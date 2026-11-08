#!/usr/bin/env python3
"""
amd-smoke.py — minimal ROCm attach workload for accelrun's toolkit-free AMD path.

STATUS (2026-11): NEVER RUN ON HARDWARE YET — it exists so the AMD hardware
session (TODO, medium term) has its first verification-ladder step. Like the
NVIDIA smoke before its first PASS, it may carry bugs; every failure-mode
below is a hypothesis to confirm or retire on real hardware.

Uses only the ROCm driver-interface layer via dlopen: no ROCm toolkit
build, no pip, no compiler. Exercises exactly what accelrun injects:
  - /dev/kfd                (compute interface: HSA queues — created by
                             hipStreamCreate without needing a kernel)
  - /dev/dri/renderD*       (GEM buffer objects: hipMalloc / hipMemcpy)
  - libhsa-runtime64.so.1   (KFD userspace — the "libcuda" analog)
  - libamdhip64.so.1        (HIP runtime, "cudart"-thin)

Run on the AMD GPU host:
  ./accelrun run --mem 16Gi --image python:3.12-slim \
      --volume $(pwd)/tests/amd-smoke.py:/tmp/amd-smoke.py:ro \
      -- python3 /tmp/amd-smoke.py

Unlike the NVIDIA smoke there is NO kernel launch here: hand-written AMDGCN
is not practical the way hand-written PTX is, and a bidirectional DMA
round-trip already proves the full attach chain (KFD open + queue ioctl,
render-node GEM mapping, host<->device copy). Kernel-level compute is the
workload's job to prove (e.g. `ollama ps` -> "100% GPU"), same as NVIDIA.

Failure-mode map (hypotheses, keyed to what accelrun injects):
  dlopen libhsa-runtime64 fails -> host KFD userspace not mounted
                                    (oci.rs AMD lib set / host ROCm absent)
  dlopen libamdhip64 fails      -> same, or version-skewed .so symlinks
  hipGetDeviceCount == 0        -> /dev/kfd not injected (docker inspect
                                    .HostConfig.Devices — the old probe's
                                    bug), or node group perms: non-root
                                    container user without --group-add
  hipMemGetInfo fails           -> HSA init failed: renderD missing, or
                                    seccomp blocking mmap (AMD marks
                                    seccomp=unconfined optional; try it
                                    as a diagnostic)
  hipMalloc fails               -> GEM path: renderD missing or group perms
  hipStreamCreate fails         -> KFD queue creation refused (kfd perms)
  round-trip mismatch           -> host ROCm vs image ROCm version skew
                                    (bind mounts shadow image copies)
  hipGetDeviceCount ok but gfx unsupported errors later
                                -> ROCm version does not support the GPU
                                    arch (HSA_OVERRIDE_GFX_VERSION is the
                                    user-side escape hatch, NOT settable
                                    via accelrun)
"""

import ctypes
import os
import sys
import time


def die(msg):
    print(f"\n*** FAIL: {msg}")
    sys.exit(1)


print("== accelrun env contract ==")
for k in ("ACCEL_BACKEND", "ACCEL_MEM_BYTES", "ACCEL_UNIFIED", "ACCEL_DEVICE", "ACCEL_NAME"):
    print(f"  {k:16s} = {os.environ.get(k, '<unset>')}")
if os.environ.get("ACCEL_BACKEND") == "cpu":
    print("  (cpu fallback selected — no GPU expected;"
          " make /dev/kfd + /dev/dri/renderD* visible and re-probe)")

print("\n== ROCm userspace (host libs via accelrun bind-mounts) ==")
try:
    ctypes.CDLL("libhsa-runtime64.so.1")
except OSError as e:
    die(f"dlopen('libhsa-runtime64.so.1') failed: {e}\n"
        "    -> host KFD userspace not mounted; run via accelrun on a host"
        " with a working ROCm userspace")
print("  libhsa-runtime64.so.1   : loaded")
try:
    hip = ctypes.CDLL("libamdhip64.so.1")
except OSError as e:
    die(f"dlopen('libamdhip64.so.1') failed: {e}\n"
        "    -> host HIP runtime not mounted")
print("  libamdhip64.so.1        : loaded")


def argify(fn, restype, argtypes):
    fn.restype = restype
    fn.argtypes = argtypes
    return fn


hipGetErrorString = argify(hip.hipGetErrorString, ctypes.c_char_p, [ctypes.c_int])
hipGetDeviceCount = argify(hip.hipGetDeviceCount, ctypes.c_int, [ctypes.POINTER(ctypes.c_int)])
hipSetDevice = argify(hip.hipSetDevice, ctypes.c_int, [ctypes.c_int])
hipMemGetInfo = argify(hip.hipMemGetInfo, ctypes.c_int,
                       [ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)])
hipStreamCreate = argify(hip.hipStreamCreate, ctypes.c_int, [ctypes.POINTER(ctypes.c_void_p)])
hipStreamDestroy = argify(hip.hipStreamDestroy, ctypes.c_int, [ctypes.c_void_p])
hipMalloc = argify(hip.hipMalloc, ctypes.c_int,
                   [ctypes.POINTER(ctypes.c_void_p), ctypes.c_size_t])
hipFree = argify(hip.hipFree, ctypes.c_int, [ctypes.c_void_p])
hipMemcpy = argify(hip.hipMemcpy, ctypes.c_int,
                   [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int])
hipMemset = argify(hip.hipMemset, ctypes.c_int,
                   [ctypes.c_void_p, ctypes.c_int, ctypes.c_size_t])
hipDeviceSynchronize = argify(hip.hipDeviceSynchronize, ctypes.c_int, [])


def hip(res, what):
    if res != 0:
        s = (hipGetErrorString(res) or b"?").decode(errors="replace")
        die(f"{what} -> hipError {res}: {s}")


# hipMemcpyKind (mirrors CUDA): 1 = HostToDevice, 2 = DeviceToHost
HIP_MEMCPY_H2D = 1
HIP_MEMCPY_D2H = 2

ndev = ctypes.c_int()
hip(hipGetDeviceCount(ctypes.byref(ndev)), "hipGetDeviceCount")
if ndev.value < 1:
    die("no HIP devices visible (kfd not injected, or group perms)")
print(f"  devices       : {ndev.value}")
hip(hipSetDevice(0), "hipSetDevice(0)")

# best-effort versions (mirror of the driver-version print in gpu-smoke.py)
drv = ctypes.c_int()
if hasattr(hip, "hipDriverGetVersion"):
    hip.hipDriverGetVersion(ctypes.byref(drv))
    print(f"  HIP driver    : {drv.value}")

free_b = ctypes.c_size_t()
total_b = ctypes.c_size_t()
# The direct analog of cuMemGetInfo_v2 — the NVIDIA five-bug chain taught:
# query memory BEFORE anything else that could mask its failure.
hip(hipMemGetInfo(ctypes.byref(free_b), ctypes.byref(total_b)), "hipMemGetInfo")
print(f"  vram          : {total_b.value / 2**30:.2f} GiB total,"
      f" {free_b.value / 2**30:.2f} GiB free")

# Queue creation: proves the KFD compute path (create_queue ioctl) without
# needing a kernel. Analog of cuDevicePrimaryCtxRetain in gpu-smoke.py.
stream = ctypes.c_void_p()
hip(hipStreamCreate(ctypes.byref(stream)), "hipStreamCreate (KFD queue)")
print("  stream        : created (KFD queue OK)")

# ---- DMA round-trip: c[i] = 0x5A5A5A5A via memset, then exact copy-back.
N = 256
A = (ctypes.c_uint32 * N)(*range(N))
B = (ctypes.c_uint32 * N)()

d = ctypes.c_void_p()
hip(hipMalloc(ctypes.byref(d), 4 * N), "hipMalloc")

t0 = time.time()
hip(hipMemcpy(d, ctypes.cast(A, ctypes.c_void_p), 4 * N, HIP_MEMCPY_H2D), "copy A->dev")
hip(hipMemset(d, 0x5A, 4 * N), "hipMemset(0x5A)")
hip(hipMemcpy(ctypes.cast(B, ctypes.c_void_p), d, 4 * N, HIP_MEMCPY_D2H), "copy dev->B")
run_ms = 1000 * (time.time() - t0)

bad = [i for i in range(N) if B[i] != 0x5A5A5A5A]
print(f"\n== memset round-trip : {run_ms:.2f} ms ==")
print(f"  B[0..4]      : {[hex(B[i]) for i in range(5)]}")
if bad:
    die(f"memset round-trip wrong at {len(bad)}/{N} elements"
        f" (first: idx {bad[0]}, got {hex(B[bad[0]])}, want 0x5a5a5a5a)")

# exact bidirectional copy: dev -> B must equal A bit-for-bit
hip(hipMemcpy(d, ctypes.cast(A, ctypes.c_void_p), 4 * N, HIP_MEMCPY_H2D), "copy A->dev (2)")
hip(hipMemcpy(ctypes.cast(B, ctypes.c_void_p), d, 4 * N, HIP_MEMCPY_D2H), "copy dev->B (2)")
bad = [i for i in range(N) if B[i] != A[i]]
print("== copy round-trip  : " + (f"BAD at {len(bad)} elements" if bad else "256/256 exact"))
if bad:
    die(f"copy round-trip wrong at {len(bad)}/{N} elements"
        f" (first: idx {bad[0]}, got {B[bad[0]]}, want {A[bad[0]]})")

hip(hipFree(d), "hipFree")
hip(hipStreamDestroy(stream), "hipStreamDestroy")
hip(hipDeviceSynchronize(), "hipDeviceSynchronize")

print(f"""
========================================
  SMOKE TEST PASSED
  backend: {os.environ.get('ACCEL_BACKEND', '?')} ({os.environ.get('ACCEL_NAME', '?')})
  attach : toolkit-free (kfd + renderD nodes + host ROCm userspace)
  path   : HSA load, KFD queue, GEM alloc, bidirectional DMA — all live
========================================""")
