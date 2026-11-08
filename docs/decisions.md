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

## 2026 — AMD attach path completed in code (not yet hardware-verified)

Found by code review against ROCm's own toolkit-free docker recipe
(`--device /dev/kfd --device /dev/dri`), fixed blind, and argv-verified on
a synthetic accelerator (`cargo test amd_dry_run -- --nocapture`; unit
tests for the APU classification). **No AMD hardware was available** —
every fix below is confirmed at the argv/logic level only; the hardware
session (TODO, medium term) owns the first PASS, and the runbook's [amd]
failure-map rows are hypotheses until then.

- **`/dev/kfd` now injected** (the old probe presence-checked it but passed
  only renderD nodes — compute would have been broken in-container; kfd is
  THE compute interface, renderD alone is the AMD "No devices were found").
- **`--group-add <gid>`** for device-node groups, gids stat'd from the
  nodes themselves (the `render` gid is not standardized): docker's
  `--device` relaxes only the cgroup side, file perms win (AMD nodes are
  0660 root:video/render; NVIDIA's 0666 — which is why the NVIDIA path
  never needed it; the fix is inert there).
- **Probe is sysfs-first** (in-tree amdgpu: `mem_info_vram_total` /
  `mem_info_gtt_total` in bytes; rocm-smi CSV drifts across ROCm releases
  and is demoted to last-resort fallback) with a **real APU heuristic**
  (PCI class `0x0380xx`, or a <4 GiB VRAM carve-out signature). The old
  `unified = render_nodes.is_empty()` was inverted in practice — APUs
  have renderD nodes, so it could never fire. Unified (APU) capacity is
  now GTT (the system-RAM pool the GPU allocates from), discrete is VRAM.
- **ROCm userspace bind-mount set** mirrors the NVIDIA pattern:
  hsa-runtime64/hsakmt (KFD userspace — the "libcuda" analog), amdhip64
  ("cudart"-thin), comgr (the ptxjit analog), drm/drm_amdgpu (GEM),
  elf/numa (deps slim images lack); identity + multiarch mirror +
  merge-usr dedup; host rocminfo/amd-smi/rocm-smi mounted like nvidia-smi.
- **Backend gating in oci.rs**: NVIDIA libs/env only on the nvidia
  backend — a custom accelerator no longer gets junk NVIDIA mounts;
  group-add is backend-generic; custom backends stay nodes-only
  (image-borne userspace, per the Gemmini case-study plan's P2).
- **Known hazard, documented rather than fixed**: the bind mounts SHADOW
  a rocm-carrying image's own stack (rocm/*, rocm/pytorch) — keep host
  ROCm >= image ROCm or strip the binds from a --dry-run argv. AMD's own
  container runtime mounts the *whole* host ROCm tree, so the subset is
  the smaller intervention.
- **Deliberately not done**: no `seccomp=unconfined` by default (AMD marks
  it optional; security posture — and no `--security-opt` passthrough
  exists yet, a documented gap); no per-GPU selection (whole-host, like
  NVIDIA's all-nodes passthrough); no kernel launch in
  `tests/amd-smoke.py` (hand-written AMDGCN is impractical the way PTX
  is; DMA round-trip + KFD queue creation is the honest equivalent) — and
  that smoke test has NEVER run on hardware, the same status
  `gpu-smoke.py` had before its first PASS.

## 2026-11 — model-conformance fixes (code-review session)

The reference implementation contradicted the model in three places; all
fixed so the demo no longer undercuts the proposal. The README now states
the positioning plainly: the model is the contribution, the launcher is its
reference implementation.

- **Selection policy now IS "CPU only when no accelerator exists."** The
  old `viable = mem_total >= budget` filter let *CPU RAM* win silently when
  a GPU missed the budget but host RAM met it (the planned NV4ads
  `--mem 8Gi` case: silent CPU; even on the verified host, `--mem 20Gi` →
  silent CPU). New tiers in `select.rs`: accelerators-meeting-budget →
  accelerators-missing-budget (loud `degraded` warning naming backend and
  gap) → CPU. Unit tests pin each tier. The warning no longer claims "fell
  back to CPU" when the degraded target is a real GPU (the P0 mislabel).
- **Unified-memory enforcement moved to the real levers.** `sys.rs`'s cgroup
  writer capped the *CLI's own process* (containers get cgroups from
  dockerd, so the cap never reached the workload) and printed a fictional
  "capped unified RAM" success; removed. Direct mode enforces via docker
  `--memory`, `--cdi` mode via the spec's `cgroupRules` (already correct in
  `cdi.rs`).
- **`docker --memory` is now scoped to unified backends only.** The old
  unconditional flag silently hard-capped container *host RAM* at the VRAM
  budget — `--mem 2Gi` on a GPU host was a 2-GiB-RAM container — and
  contradicted the README's "hint, not a cap". Behavior change vs the
  verified baseline: containers on discrete GPUs now run with uncapped
  host RAM; nothing verified depended on the cap (unified behavior is
  unchanged — still capped).
- **`ProbeFalseNegative` detection is backend-generic** (was nvidia-smi
  only, so an AMD probe miss was structurally invisible): fires when
  selection landed on CPU while `nvidia-smi -L` lists a GPU OR `/dev/kfd`
  exists. Custom TOML probes have no presence signal — documented gap.
- **`UnifiedHostOOM` detection reads `/proc/vmstat` `oom_kill`** (host-level
  kernel counter) instead of the `memory.events` of a cgroup that was
  never the container's (always 0). Mitigation text updated to match.
- Docs synced: README (positioning paragraph, `--mem` bullet,
  `ACCEL_MEM_BYTES` row), vision.md (enforcement mechanism, architecture
  diagram), runbook (APU note, AMD ladder step 3, NV4ads session step 4
  expectation, cleanup inventory), clean.sh inventory (legacy cgroup
  wording). amd-smi noted as a python tool in the oci.rs mount comment.
- **Rootless-container build recipe extended** (next no-`cc` dev session,
  take note): `cargo build` works with the documented fakelib workaround
  as-is, but `cargo test` (libtest) also needs `-lgcc_s` — symlink the
  system `libgcc_s.so.1` into the fakelib — and the extracted `libc.so`
  ld-script's absolute `/lib`/`/usr/lib` paths repointed into the tree.
  With those two additions the full suite (9 tests) builds and passes
  in a container with no C toolchain.
- **Not done here, deliberately**: no `--security-opt` passthrough, no
  per-GPU selection, no change to `--mem` semantics for unified backends.
