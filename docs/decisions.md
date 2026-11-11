# Decisions log

> **AI-assisted development, human-verified.** accelrun's code, scaffolding,
> documentation, and artifacts were produced with LLM-based coding agents
> under the author's direction. All design decisions, review, and hardware
> verification are human work — including catching the bugs the generated
> code carried (the five-bug chain below). Every entry is a verified record,
> kept as the project's technical memory.

## Foundational decisions

- **Rust with zero external crates.** CLI parsing, JSON, a TOML subset, and
  glob are hand-rolled: the release binary stays ≈460 KB and builds anywhere —
  including rootless containers with no C toolchain (the first dev
  environment had no system `cc`; the host-specific linker workaround stays
  out of the repo, see `.gitignore`).
- **Abstract the resource, never emulate CUDA.** Portability lives at the
  workload's IR level (Triton / MLIR / SYCL / WebGPU / ONNX-RT); the launcher
  only abstracts the *claim* (`ACCEL_*` contract).
- **Direct injection by default, CDI opt-in.** Plain `--device` + `-e` works
  on any Docker version; `--device cdi:accel.local/gpu=gpu0` (Docker ≥ 25) is
  for fleet tooling that wants vendor-neutral specs.
- **Toolkit-free attach.** Numbered `/dev/nvidia*` + control nodes, driver
  libs bind-mounted read-only and mirrored into `/usr/lib/x86_64-linux-gnu`
  for cross-distro loader paths, host `nvidia-smi` mounted in. `--gpus all`
  is a last resort (WSL-like hosts) with a toolkit-presence check and a
  clear remediation error.
- **Selection policy.** Highest bandwidth among backends meeting the
  budget; graceful degradation (a real GPU beats CPU even when short on
  VRAM); `--unified` doubles the score of unified-memory backends.
- **`--mem` is a hint, not a cap.** No hard VRAM capping without MIG —
  `ACCEL_MEM_BYTES` is the app-level contract. Alpine/musl unsupported for
  NVIDIA userspace (host driver binaries are glibc).
- **Fault model.** FMEA-style registry: ProbeFalseNegative, UnifiedHostOOM,
  VramOvercommit, BackendCrashMidRun, NumericDrift — with detection hooks
  (`accelrun faults`).
- **Scope.** Single-node launcher; the emitted CDI spec is the integration
  surface for orchestrators (a k8s CRI plugin would break zero-dep and
  re-enter nvidia-device-plugin territory).
- **Docker build path** for hosts without Rust; release profile (`lto`,
  `strip`, `codegen-units=1`) keeps the binary tiny; `Cargo.lock` committed
  for reproducible builds.

Bug-driven evolution before the first full pass: docker rejects JEDEC
`--memory` suffixes (pass raw bytes); CPU fallback outranking GPU (prefer
any real accelerator, rank by bandwidth first); missing toolkit producing
cryptic docker errors (presence check + remediation); `run` keyword made
optional; `--dry-run` added as the first debugging tool; `nvidia-smi`
bind-mounted for non-CUDA images.

## The five-bug chain — first end-to-end GPU pass

The first complete pass on real hardware (Fedora host, RTX 5070 Ti 16 GB,
driver 580 open kernel module, **no NVIDIA Container Toolkit**) surfaced a
chain of five bugs, each masked by the one above it. It is kept in full as
the canonical example of why the verification ladder exists: until the
smoke test reached PASS once in its life, it was hiding its own bugs.

1. **Glob class parser.** `[0-9]` was assumed two-char → never matched →
   `glob_dev("/dev/nvidia[0-9]*")` always empty → only control nodes were
   injected → in-container `nvidia-smi`: "No devices were found".
2. **Cross-distro libpath.** Identity bind of `/usr/lib64` is invisible to
   the Debian-family loader → `dlopen("libcuda.so.1")` failed. Fix: mirror
   driver libs into `/usr/lib/x86_64-linux-gnu`.
3. **Smoke-test context.** The CUDA context was created *after* the first
   context-using call → CUresult 201. Never hit earlier because the script
   always died at dlopen first.
4. **False lead, retracted.** "Host CUDA broken / stale nvidia_uvm" — the
   host was healthy: modules matched, dmesg clean, root behaved like user.
5. **Legacy v1 symbols lie.** Unversioned `cuMemGetInfo` returns 201 on
   driver 580 even on a healthy host and context; every real binding must
   use `*_v2` (`cuMemGetInfo_v2`, `cuMemAlloc_v2`, `cuMemcpy*_v2`,
   `cuMemFree_v2`).

Lessons: (a) a smoke test must reach PASS once in its life, or it hides its
own bugs; (b) bisect host-vs-container with the SAME snippet; (c) never call
legacy unversioned CUDA memory entry points; (d) "it worked before" can mean
a workload silently serving on CPU. All of it is distilled into the
[runbook](runbook.md) failure map and verification ladder.

## 2026 — promotion to standalone repository

- accelrun started as the launcher for one of the author's own private
  projects and is promoted to a standalone, public, dual-licensed
  (MIT OR Apache-2.0) repository now that it is production-verified
  end-to-end.
- The session-by-session worklog was distilled into the sections above; the
  full original stays in the author's local archive (gitignored).
- Operator-facing knowledge lives in the [runbook](runbook.md); design
  rationale in [vision.md](vision.md).
- **Citation metadata: single license string for Zenodo.** Zenodo's GitHub
  DOI importer (invenioRDM, since late 2023) requires `CITATION.cff`
  `license` as a single SPDX string and crashes on the spec-valid array
  form with `Citation metadata load failed` (zenodo/zenodo#2515, open as
  of 2026-11; cffconvert 1.3.x trips on arrays the same way). Decision:
  the CFF carries `license: MIT`; the dual grant stays declared by
  LICENSE-MIT + LICENSE-APACHE + Cargo.toml's `MIT OR Apache-2.0`
  expression; the Zenodo record description states the dual license.
  Restore the array if #2515 is ever fixed.

## 2026-11 — WSL2 direct attach (dxg), toolkit-free

Motivation: a WSL2 smoke run landed on the CPU backend (contract printed,
exit 1 was gpu-smoke.py's own `die()` on dlopen — accelrun behaving as
designed) because two probe assumptions break on WSL2: numbered
`/dev/nvidia*` nodes never exist there (the GPU is the paravirt `/dev/dxg`
node), and nvidia-smi lives at `/usr/lib/wsl/lib/` which non-Ubuntu distros
leave off PATH, so the PATH-only `Command::new("nvidia-smi")` smi-fallback
never fired. The run also exposed a wrong claim in oci.rs's `--gpus all`
error path — "On WSL this is unavoidable (docker needs the toolkit)" —
which is false: the toolkit's own WSL mode is just `/dev/dxg` +
`/usr/lib/wsl/lib` + `LD_LIBRARY_PATH`, all of which accelrun can inject
itself.

- **Probe (nvidia.rs)**: new `smi()` helper tries `/usr/lib/wsl/lib/nvidia-smi`
  first, then PATH. `detect()` keeps the bare-metal path untouched; when
  numbered nodes are absent but `/dev/dxg` exists AND smi confirms an
  NVIDIA GPU (dxg alone is vendor-neutral DirectCompute and proves
  nothing), devices becomes `["/dev/dxg"]` → direct attach.
  Driver-present-but-no-nodes-and-no-dxg (restricted namespaces) still
  degrades to the `--gpus all` last resort exactly as before.
- **Attach (oci.rs)**: the dxg node rides the existing generic `--device`
  loop; the attach gains a WSL branch mirroring libnvidia-container's WSL
  mode: `/usr/lib/wsl/lib` bind-mounted `:ro` at its identity path +
  `-e LD_LIBRARY_PATH=/usr/lib/wsl/lib` + smi mounted to `/usr/bin` for
  parity with the bare-metal path. The bare-metal argv sits unchanged in
  the `else` branch. The corrected error text names the real remediations
  (Windows driver → dxg appears; toolkit for `--gpus all`).
- **Known caveats** (runbook WSL section + failure map): `docker -e` cannot
  append, so an image-set `LD_LIBRARY_PATH` is overridden; Docker Desktop's
  daemon lives in its own distro, so `--device /dev/dxg` and the wsl-lib
  bind resolve in that VM's filesystem — the supported path is docker
  native to the WSL distro; the driver must be the Windows-side one (a
  Linux driver inside WSL is unsupported and breaks the wsl-lib mount).
- **Verification status**: argv-verified on a synthetic dxg accelerator —
  `nvidia_wsl_dry_run_argv_assembles` is this branch's only unit test
  (main predates the test suite). NEVER run on WSL hardware. The patch was
  developed on the amd_support-stacked tree, where the full 10-test suite
  passed with it applied; this branch is the same patch re-authored
  standalone (`git diff main wsl_patch`), re-built and argv-eyeballed
  here. The select/sys suites do not exist on this branch — they belong
  to the amd_support changeset.
- **Branch note**: standalone feature branch off `main` (cf6fb66), parallel
  to `amd_support`, so the AMD and WSL lines of work stay independent.
  Merging both reproduces the stacked tree the patch was developed on —
  that tree is the recorded, suite-tested resolution of the oci.rs
  conflict between the two branches.
- **No-`cc` dev-container recipe, second extension** (build infra, applies
  repo-wide): the fakelib needs BOTH `libgcc_s.so.1` (runtime name — what
  the earlier recipe recorded) AND `libgcc_s.so -> system libgcc_s.so.1`
  (the LINKER name: lld resolves `-lgcc_s` against `libgcc_s.so`, which
  is why a `.so.1`-only symlink fails the test build).
