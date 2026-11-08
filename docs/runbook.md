# accelrun RUNBOOK — launching and troubleshooting GPU containers

How to launch, what can go wrong, and how to find out — distilled from the
first full end-to-end deployment (Fedora host, RTX 5070 Ti, driver 580.178.04
open kernel module, no NVIDIA Container Toolkit, ollama workload).

## TL;DR — the launch modes

| Mode | When | Requirements |
|---|---|---|
| **direct** (default) | bare-metal `/dev/nvidia*` nodes, or WSL2 (`/dev/dxg` + Windows driver) | none: any docker |
| **cdi** (`--cdi`) | fleet tooling wants vendor-neutral specs | docker ≥ 25, spec in the daemon CDI dir |
| `--gpus all` (last resort) | driver works but neither numbered nodes nor `/dev/dxg` visible (restricted namespaces) | NVIDIA Container Toolkit registered in the daemon |
| (non-accelrun) host-native service | no GPU in containers at all | host systemd service + `host-gateway` networking |

Direct mode injects: numbered device nodes + control nodes (`nvidiactl`,
`nvidia-uvm`, `nvidia-uvm-tools`), the driver libraries bind-mounted both at
their host path **and** mirrored into `/usr/lib/x86_64-linux-gnu` (cross-distro
loader paths), host `/usr/bin/nvidia-smi`, and the env contract (`ACCEL_*`).
On WSL2 the same direct mode takes the WSL shape (`/dev/dxg` +
`/usr/lib/wsl/lib`) — see
[WSL2 attach](#wsl2-attach-implemented-hardware-verification-pending).

## Command reference

```
accelrun probe                    # detected backends + diagnostics
accelrun faults                   # FMEA registry

accelrun [run] --mem 14Gi [--unified] [--name N] [--network NET]
              [-d] [-p HOST:PORT]... [-e KEY=VAL]... [--restart POLICY]
              [--cdi] [--dry-run] [--volume SRC:DST[:ro]]...
              --image IMG -- CMD...
```

- `run` is optional: bare flags are treated as run.
- `--mem` is a budget HINT (Gi/Mi/raw bytes): selection degrades gracefully
  (no hard VRAM cap without MIG — `ACCEL_MEM_BYTES` is an app-level contract).
- `-d`/`--restart` drop `--rm` (docker refuses `--rm` + `--restart`): failed
  or one-shot detached containers stay behind — remove with `docker rm`.
- `--dry-run` prints the exact docker argv. **First** debugging tool.

## Verified recipe: ollama as a named GPU peer on a compose network

```bash
docker network create accel
./accelrun run --mem 14Gi --name ollama --network accel -d \
    --restart unless-stopped -p 11434:11434 \
    --image ollama/ollama:latest \
    --volume ollama-data:/root/.ollama \
    -- serve
docker exec ollama ollama pull qwen2.5:14b
docker exec ollama ollama ps          # PROCESSOR must say "100% GPU"
```

Gotcha — images with an ENTRYPOINT prefix the container command (ollama's is
`/bin/ollama`): pass just `serve`, not `ollama serve`; `-- nvidia-smi`
becomes `ollama nvidia-smi` and dies with "unknown command". For one-shots
use an ENTRYPOINT-less image (`python:3.12-slim`) or `docker exec` into a
running service container.

## Verification ladder (in this order)

1. Host `nvidia-smi` — necessary, **not** sufficient (never touches CUDA/UVM).
2. Host CUDA probe with `*_v2` symbols (below) — the TRUE host check.
3. `accelrun probe` — backend + diagnostics sane.
4. `--dry-run` — the argv you expect (devices, libs, env)?
5. `docker inspect <c> --format '{{json .HostConfig.Devices}}'` — what the
   container really got: numbered nodes + control nodes?
6. In-container `nvidia-smi -L` (glibc images only).
7. `tests/gpu-smoke.py` on `python:3.12-slim` — real compute + PTX JIT: the
   definitive verdict (recipe in README).
8. Workload check: e.g. `ollama ps` → "100% GPU".

Host CUDA probe (ended a two-day hunt — use `_v2`, see failure map):

```bash
python3 -c "
import ctypes
c = ctypes.CDLL('libcuda.so.1')
dev = ctypes.c_int(); ctx = ctypes.c_void_p(); f = ctypes.c_size_t(); t = ctypes.c_size_t()
c.cuInit(0); c.cuDeviceGet(ctypes.byref(dev), 0)
c.cuDevicePrimaryCtxRetain(ctypes.byref(ctx), dev); c.cuCtxSetCurrent(ctx)
print('v2 ->', c.cuMemGetInfo_v2(ctypes.byref(f), ctypes.byref(t)))
print('GiB =', t.value/2**30)"
```

## WSL2 attach (implemented; hardware verification pending)

WSL2 never creates numbered `/dev/nvidia*` nodes: the GPU is the paravirt
`/dev/dxg` node, and the **Windows** driver ships the userspace (libcuda,
NVML, PTX JIT, nvidia-smi) under `/usr/lib/wsl/lib` inside the distro.
Never install a Linux NVIDIA driver inside WSL — the Windows driver is the
only supported source. Direct mode therefore takes the WSL shape (mirrors
nvidia-container-toolkit's own WSL mode, still toolkit-free):

- `--device /dev/dxg` — the paravirt compute interface
- `/usr/lib/wsl/lib` bind-mounted read-only at its identity path
- `LD_LIBRARY_PATH=/usr/lib/wsl/lib` so the container's loader and
  `dlopen("libcuda.so.1")` find the driver (glibc images only, as ever —
  no alpine/musl)
- `nvidia-smi` mounted at `/usr/bin/nvidia-smi`, parity with bare metal

The probe claims the GPU only when BOTH `/dev/dxg` exists AND nvidia-smi
(also tried at `/usr/lib/wsl/lib/nvidia-smi` — non-Ubuntu distros leave it
off PATH) confirms an NVIDIA GPU: dxg alone is vendor-neutral
DirectCompute and proves nothing. Requirements and caveats:

- docker must run **native to the WSL distro** (e.g. `apt install docker.io`
  in the distro): with Docker Desktop the daemon lives in its own
  `docker-desktop` distro, so `--device /dev/dxg` and the wsl-lib bind
  resolve in that VM's filesystem — untested there; use the toolkit path
  if anything.
- docker `-e` cannot append: an image that pre-sets `LD_LIBRARY_PATH` gets
  it overridden by the attach.
- the first `gpu-smoke.py` PASS on WSL hardware is still pending — the
  argv is verified on a synthetic dxg accelerator only (decisions log,
  2026-11 entry).

WSL verification ladder (in the distro):

1. `uname -r` ends in `-microsoft-standard-WSL2` (WSL1 has no GPU support).
2. `ls -l /dev/dxg` — missing → install the NVIDIA driver **on Windows**
   (any recent Game Ready/Studio driver includes WSL CUDA); verify
   `nvidia-smi` on the Windows side.
3. `/usr/lib/wsl/lib/nvidia-smi -L` lists the GPU; `./accelrun probe` shows
   the `nvidia` line with `devices=["/dev/dxg"]` plus the dxg diagnostic.
4. `./accelrun run --mem 8Gi --image python:3.12-slim --dry-run -- true` —
   the argv must carry `--device /dev/dxg`, the wsl-lib mount, and
   `LD_LIBRARY_PATH`.
5. `gpu-smoke.py` (README recipe) — the definitive verdict.

## Failure map

Unmarked lines are bugs we actually hit; lines marked **[wsl]** are
*hypotheses* from the 2026-11 code session — the WSL2 hardware session
confirms or retires them (the attach path has never run on WSL silicon).

| Symptom | Root cause | Fix |
|---|---|---|
| in-container nvidia-smi: "No devices were found" | numbered `/dev/nvidia[0-9]*` not injected (glob class parser bug; or WSL-like host with only control nodes) | inspect `HostConfig.Devices`; fixed parser; WSL hosts → direct dxg attach (WSL2 section above) |
| `dlopen("libcuda.so.1")` fails in a Debian-family image (Fedora host) | identity bind of `/usr/lib64` invisible to the multiarch loader | libs mirrored into `/usr/lib/x86_64-linux-gnu` — keep that in any future lib logic |
| exit 127, "nvidia-smi: not found" on alpine | musl cannot exec the host glibc binary | never use alpine/musl for NVIDIA userspace; `python:3.12-slim` |
| **[wsl]** probe prints `cpu` despite a GPU on the Windows side | PATH-only nvidia-smi lookup missed `/usr/lib/wsl/lib` (non-Ubuntu distros keep it off PATH) — fixed 2026-11-08: the probe tries both — or no Windows driver, so no `/dev/dxg` at all | install the NVIDIA driver **on Windows**; `accelrun probe` diagnostics name both smi paths it tries |
| **[wsl]** in-container `dlopen("libcuda.so.1")` fails | wsl-lib mount or `LD_LIBRARY_PATH` missing from the argv; or the image pre-set `LD_LIBRARY_PATH` and the attach overrode it; or (Docker Desktop) the daemon resolved the bind in its own VM | dry-run must show `-v /usr/lib/wsl/lib:...` + `-e LD_LIBRARY_PATH=...`; use docker native to the distro |
| **[wsl]** cuInit / device count fails though dxg is injected | old Windows driver, or the daemon could not stat `/dev/dxg` | WSL ladder steps 2–4 above |
| "unknown command 'X' for 'Y'" | image ENTRYPOINT prefixes the container command | pass just the subcommand (ollama: `serve`) or use `exec` |
| container name / port conflicts on relaunch | with `-d` there is no `--rm`: failed containers persist | `docker rm -f <name>` before relaunching |
| `cuMemGetInfo` → 201 with a healthy context, on host AND container, user AND root | legacy unversioned v1 symbol: driver 580 returns `CUDA_ERROR_INVALID_CONTEXT` from it even on a healthy machine | always call `*_v2` (`cuMemGetInfo_v2`, `cuMemAlloc_v2`, `cuMemcpy*_v2`, `cuMemFree_v2`) |
| models re-download after a relaunch | volume name typo — docker silently auto-creates unknown volumes | `docker volume ls`; reuse the exact name |
| `ollama ps` empty right after start | normal: models load on demand | run one prompt first, then `ps` |
| workload "works" but slowly | silent CPU fallback (ollama's specialty) | `ollama ps` PROCESSOR column / in-container `nvidia-smi` during generation |
| nvidia-smi fine, CUDA broken | nvidia-smi never uses UVM/contexts | host probe (ladder step 2); only a 201 from `_v2` deserves a driver hunt |

## Bisection methodology (when a container misbehaves)

1. **Same snippet, host vs container** — the container faithfully mirrors
   host state; identical failure on the host = not an injection problem.
2. `docker inspect .HostConfig.Devices` — the truth about container creation.
3. `--dry-run` — the truth about the argv accelrun assembles.
4. `/proc/<pid>/fd` (add a `sleep` to the probe) — which device nodes a
   process actually opened. No strace required.
5. `dmesg` for NVRM/Xid/uvm lines — RM-level failures log there.

## Host requirements (the boring checklist)

- driver + device nodes: `ls /dev/nvidia*` → `nvidia0`, `nvidiactl`,
  `nvidia-uvm` (`nvidia-uvm` is created on demand by udev at first CUDA use)
- `lsmod | grep nvidia` shows `nvidia_uvm`; `nvidia-smi` OK
- userspace driver libs in `/usr/lib64` (or the Debian multiarch dirs)
- Blackwell GeForce (RTX 50xx): open kernel module — the proprietary module
  does not support consumer Blackwell
- docker with **no** special runtime config: toolkit-free by design
- WSL2: no `/dev/nvidia*` ever — `/dev/dxg` + `/usr/lib/wsl/lib` are the
  equivalents (driver = the **Windows** one; never install a Linux driver
  inside WSL); docker must be native to the distro

## Known gaps

- no `--entrypoint` passthrough (one-shot commands in ENTRYPOINT images)
- no hard VRAM cap (hint only; consumer cards have no MIG)
- single-node launcher; the CDI spec is the k8s integration surface
- Alpine/musl unsupported for NVIDIA userspace
- WSL2 attach path is implemented but **not hardware-verified** — everything
  above marked [wsl] is a hypothesis until the WSL session
