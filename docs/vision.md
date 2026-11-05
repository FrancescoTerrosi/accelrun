# accelrun — portable GPU/CPU resource abstraction for containers

## Vision
Run the same ML workload **transparently** across:
- CPU-only
- NVIDIA GPU (discrete VRAM)
- AMD GPU / APU (discrete or unified)
- Apple Silicon (Metal, unified LPDDR)
- Custom accelerators (Tenstorrent, Habana, Groq, ...)

The container claims an **abstract accelerator resource** (`accel:1`, `accel-mem:16Gi`),
never a physical device. accelrun probes the host, picks the best backend,
and injects devices / env vars / memory caps via direct argv injection
(`--device`, read-only bind mounts, `-e`) or a vendor-neutral CDI device.

## Guiding force
**Portability**, not raw performance. CPU fallback is acceptable (slow but correct).

## Key design decisions
1. Abstract at the **resource/claim level** (CDI + OCI + cgroups), NOT by emulating
   CUDA. Workloads stay portable via Triton / MLIR / SYCL / WebGPU / ONNX-RT.
2. **Unified memory** (Apple UMA, AMD APU, CXL pools) is a first-class backend:
   cap it with cgroup v2 `memory.max`, since there is no hardware VRAM limit.
3. Discrete VRAM on consumer NVIDIA cannot be hard-capped without MIG — expose
   the budget as an app-level contract (`ACCEL_MEM_BYTES`) instead. Documented gap.
4. Safety/RAMS layer: FMEA-style fault registry with
   detection hooks and mitigations (backend misdetection, host OOM on unified RAM,
   numeric drift across backends, mid-run device crash).

## Architecture
```
App (Triton/MLIR/SYCL/WebGPU portable code)
        │ reads env: ACCEL_BACKEND, ACCEL_NAME, ACCEL_MEM_BYTES,
        │             ACCEL_UNIFIED, ACCEL_DEVICE
        ▼
accelrun run --mem 16Gi --image X -- cmd      (probe → select → inject)
        │
        ├─ direct (default, any Docker): ACCEL_* env + raw /dev/nvidia*
        │  nodes + control nodes + driver libs bind-mounted :ro
        └─ cdi (opt-in, Docker ≥ 25):    --device cdi:accel.local/gpu=gpu0
           (spec emitted every run — the orchestrator integration surface)
        ▼
Backend: nvidia | amd | metal | custom | cpu
        (+ best-effort cgroup v2 memory.max cap on unified backends;
         FMEA detection hooks throughout)
```

## Open gaps in SotA this project addresses
(Evidence and WERE/ARE status for each: [sota-gaps.md](sota-gaps.md))
- No vendor-neutral accelerator *resource class* in OCI/CDI (only per-vendor devices).
- Container runtimes are VRAM-centric; none understand unified-memory pools.
- No cgroup controller for "GPU memory" inside unified RAM.
- Portable-GPU projects ignore failure semantics (no fault model / RAMS).

## References
- OCI runtime spec + CDI (Container Device Interface) v0.8+
- cgroup v2 memory controller
- Linux HMM / CUDA Unified Memory / ROCm HMM for unified access
- Triton, MLIR/IREE, SYCL 2020, WebGPU (Dawn), ONNX Runtime EPs
