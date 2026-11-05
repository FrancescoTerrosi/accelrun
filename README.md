# accelrun

**Run GPU workloads in containers with no NVIDIA Container Toolkit, no runtime registration, and any Docker version.**

`accelrun` is a single zero-dependency Rust binary (~460 KB) that probes your host, picks the best accelerator for a requested memory budget, and launches containers with the GPU passed through as raw device nodes plus bind-mounted driver libraries. Your workload stays portable across NVIDIA / AMD / Apple Metal / custom accelerators / CPU via a vendor-neutral environment contract (`ACCEL_*`) — and optionally as a vendor-neutral CDI device.

## Quickstart

Requirements: a Linux host with **Docker** and a **working GPU driver** (`nvidia-smi` OK on the host). accelrun does not install drivers, runtimes, or toolkits — that is the point. No GPU? You get graceful CPU fallback instead.

Build the binary (no Rust toolchain needed):

```bash
git clone https://github.com/FrancescoTerrosi/accelrun.git && cd accelrun
docker build -t accelrun:latest .
docker create --name ar accelrun:latest \
  && docker cp ar:/usr/local/bin/accelrun . && docker rm ar
chmod +x accelrun
```

Or with Rust installed: `cargo build --release` (binary at `target/release/accelrun`).

See what your host has:

```console
$ ./accelrun probe
nvidia   RTX 5070 Ti        mem= 17094934528 BW=  1000 GB/s unified=false devices=["/dev/nvidia0", "/dev/nvidiactl", ...]
cpu      CPU x4             mem= 33402966016 BW=    50 GB/s unified=true  devices=[]
--- diagnostics ---
...driver, device-node, and loader details used for troubleshooting...
```

Run a container claiming a 12 GiB accelerator:

```console
$ ./accelrun run --mem 12Gi --image python:3.12-slim -- \
    python -c "import os; print(os.environ['ACCEL_BACKEND'], os.environ['ACCEL_MEM_BYTES'])"
nvidia 12884901888        # on a GPU host
cpu    12884901888        # on a machine without a GPU (warning on stderr)
```

The GPU is really in the container — verify with real CUDA compute (driver API only: no CUDA toolkit, no pip, no compiler):

```bash
./accelrun run --mem 12Gi --image python:3.12-slim \
    --volume $(pwd)/tests/gpu-smoke.py:/tmp/gpu-smoke.py:ro \
    -- python3 /tmp/gpu-smoke.py
```

If that prints `PASS`, you have PTX-JIT'd vector-add running in a container launched with zero toolkit involvement.

**Status: production-verified end-to-end** on a bare-metal Fedora host (RTX 5070 Ti, driver 580 open kernel module, no NVIDIA Container Toolkit): the smoke test above plus a 14B LLM served at "100% GPU" per `ollama ps`, in an accelrun-launched container. Other backends (AMD ROCm, Apple Metal, custom TOML probes) are detected, but NVIDIA is the only one verified end-to-end so far.

## What it is — and what it is not

**It is** a single-node, zero-dependency launcher: probe → select → inject → `docker run`. Portability lives in the workload (Triton / MLIR / SYCL / WebGPU / ONNX-RT), exposed to it via the `ACCEL_*` contract.

**It is not:**
- **CUDA emulation** — portability is at the workload's IR level (same model as LLVM/XLA), not in the launcher.
- **Hard VRAM capping** — `--mem` is a budget hint (`ACCEL_MEM_BYTES`); consumer cards have no MIG, so capping is left as a documented gap.
- **Multi-node / k8s scheduling** — the emitted CDI spec is the integration surface for orchestrators.
- **Alpine/musl** for NVIDIA userspace — host driver binaries are glibc; use glibc images such as `python:3.12-slim`.

## How the GPU gets into the container

| Mode | When it engages | Requirements |
|---|---|---|
| **direct** (default) | bare-metal host with `/dev/nvidia*` nodes | none — works on any Docker |
| **cdi** (`--cdi` flag) | fleet tooling wants vendor-neutral specs | Docker ≥ 25, spec in the daemon CDI dir |
| `--gpus all` (automatic last resort) | driver works but no numbered nodes are visible (WSL-like hosts) | NVIDIA Container Toolkit — accelrun errors with remediation if absent |

Direct mode injects, with no toolkit involved:

- numbered `/dev/nvidia*` device nodes **and** control nodes (`nvidiactl`, `nvidia-uvm`, `nvidia-uvm-tools`)
- the full driver-library set (`libcuda`, NVML, PTX JIT, compiler, ...) bind-mounted read-only — mirrored into `/usr/lib/x86_64-linux-gnu` so Debian/Ubuntu images' loader finds them
- host `nvidia-smi` bind-mounted into the container
- the `ACCEL_*` environment contract, plus `NVIDIA_VISIBLE_DEVICES=all` and `NVIDIA_DRIVER_CAPABILITIES=compute,utility`

Use `--dry-run` at any time to print the exact `docker` argv accelrun would run — the first debugging tool.

## The `ACCEL_*` contract

What the container sees, regardless of the backend that served it:

| Variable | Meaning |
|---|---|
| `ACCEL_BACKEND` | selected backend: `nvidia` \| `amd` \| `metal` \| `custom` \| `cpu` |
| `ACCEL_NAME` | accelerator name as probed (`RTX 5070 Ti`, `tenstorrent`, ...) |
| `ACCEL_MEM_BYTES` | requested memory budget (bytes) — a hint for the app, not a hard cap |
| `ACCEL_UNIFIED` | `true` if memory is unified with host RAM (APU, Apple, CPU) |
| `ACCEL_DEVICE` | primary device node (empty when backend has none) |

Optionally, the same resource is published as a vendor-neutral CDI device — `--device cdi:accel.local/gpu=gpu0` — with the spec written under `/tmp/accelrun-cdi` for inspection and installation into the daemon's CDI directory.

## CLI reference

```
accelrun probe                          # detected backends + host diagnostics
accelrun faults                         # FMEA-style fault registry (probe misses, OOM, drift, ...)

accelrun [run] --mem SIZE [--unified] [--name N] [--network NET]
              [-d] [-p HOST:PORT]... [-e KEY=VAL]... [--restart POLICY]
              [--cdi] [--dry-run] [--volume SRC:DST[:ro]]...
              --image IMG -- CMD...
```

- `run` is optional: bare flags are treated as `run`.
- `--mem` accepts `Gi` / `Mi` / raw bytes; if no backend meets the budget, the best accelerator is picked anyway (bandwidth-ranked, real GPU preferred over CPU) — CPU only when no accelerator exists.
- `-d` and `--restart` drop `--rm`, so failed or one-shot detached containers stay behind — remove with `docker rm`.
- ENTRYPOINT-bearing images prefix `CMD` (e.g. pass `serve` for `ollama/ollama`, not `ollama serve`). For one-shots prefer ENTRYPOINT-less images like `python:3.12-slim`.

A verified recipe — ollama as a named GPU peer on a compose network:

```bash
docker network create accel
./accelrun run --mem 14Gi --name ollama --network accel -d \
    --restart unless-stopped -p 11434:11434 \
    --image ollama/ollama:latest \
    --volume ollama-data:/root/.ollama \
    -- serve
docker exec ollama ollama pull qwen2.5:14b
docker exec ollama ollama ps    # PROCESSOR must say "100% GPU"
```

## Custom accelerators

Register exotic hardware (Tenstorrent, CXL pools, ...) without recompiling — drop a TOML file in `~/.config/accelrun/probes.toml` (template in [`specs/probes.toml.example`](specs/probes.toml.example)):

```toml
[[probe]]
id = "tenstorrent"
device_glob = "/dev/tenstorrent/*"
mem_gib = 16
unified = false
```

## Troubleshooting

Every failure you can hit has a mapped cause and fix — the [`runbook`](docs/runbook.md) contains a verification ladder (host probe → `--dry-run` → `docker inspect` → in-container smoke test), a failure map distilled from a real debugging session, and a bisection methodology. Start with `--dry-run` and the ladder; the map covers everything from "No devices were found" to legacy `cuMemGetInfo` lying with exit 201.

## Documentation

- [`docs/runbook.md`](docs/runbook.md) — the operator entry point: launch modes, verified recipes, verification ladder, failure map, bisection methodology
- [`docs/vision.md`](docs/vision.md) — design rationale: abstract resource claims, unified memory, the fault model
- [`docs/decisions.md`](docs/decisions.md) — the full decision log, including the five-bug chain behind the first end-to-end pass
- [`docs/releasing.md`](docs/releasing.md) — maintainer checklist: pre-push verification, tagging, releases, and the Zenodo DOI setup

## Development & AI assistance

accelrun was developed with the help of LLM-based coding agents: code, scaffolding, documentation, and test artifacts were produced with AI assistance. Every design decision, code review, and hardware verification is human work — and that split is deliberate. The project's verification philosophy exists precisely because generated code must not be trusted on faith: the [decisions log](docs/decisions.md) records the bugs the AI-assisted code carried and how hardware-level debugging caught them, including the five-bug chain behind the first end-to-end GPU pass.

## Roadmap

- podman support (the attach logic is argv-assembly; the launcher currently hardcodes `docker`)
- K8s CRI plugin variant (publishing `accel.local/gpu` as a cluster resource)
- MIG-aware NVIDIA hard memory caps
- full numeric-drift testing across backends

## License

Dual-licensed under **MIT OR Apache-2.0**, at your option — see [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
