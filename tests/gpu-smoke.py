#!/usr/bin/env python3
"""
gpu-smoke.py — minimal CUDA *compute* workload for accelrun's toolkit-free attach.

Uses only the CUDA driver API via dlopen("libcuda.so.1"): no CUDA toolkit,
no pip, no compiler. Exercises exactly what accelrun injects:
  - /dev/nvidia* + control nodes      (cuInit, context, memcpy)
  - libcuda.so.1                      (driver API)
  - libnvidia-ptxjitcompiler/nvvm/    (PTX -> native SASS JIT at module load)
    compiler                          (the libs added in the session-12 fix)

Run on the GPU host (rebuild accelrun first!):
  ./accelrun run --mem 12Gi --image python:3.12-slim \
      --volume $(pwd)/tests/gpu-smoke.py:/tmp/gpu-smoke.py:ro \
      -- python3 /tmp/gpu-smoke.py

Failure-mode map:
  dlopen fails             -> device nodes / driver libs not injected
  cuMemGetInfo fails       -> context missing (created after device
                              selection) OR legacy v1 symbol called: driver
                              580 returns 201 from unversioned cuMemGetInfo
                              even on a HEALTHY host — use cuMemGetInfo_v2
  cuModuleLoadData fails   -> PTX JIT libs missing/mismatched
  cuLaunchKernel fails     -> context/device problem
  wrong results             -> numeric bug (none expected: fp32 exact here)
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
          " lower --mem or make /dev/nvidia* visible)")

print("\n== CUDA driver API (host libcuda.so.1 via accelrun bind-mount) ==")
try:
    cuda = ctypes.CDLL("libcuda.so.1")
except OSError as e:
    die(f"dlopen('libcuda.so.1') failed: {e}\n"
        "    -> driver lib not mounted/invisible; run via accelrun on a host"
        " with visible /dev/nvidia* nodes")


def cu(res, what):
    if res != 0:
        p = ctypes.c_char_p()
        cuda.cuGetErrorString(res, ctypes.byref(p))
        s = (p.value or b"?").decode(errors="replace")
        die(f"{what} -> CUresult {res}: {s}")


cu(cuda.cuInit(0), "cuInit")

ndev = ctypes.c_int()
cu(cuda.cuDeviceGetCount(ctypes.byref(ndev)), "cuDeviceGetCount")
if ndev.value < 1:
    die("no CUDA devices visible (device nodes not passed?)")
print(f"  devices       : {ndev.value}")

dev = ctypes.c_int()
cu(cuda.cuDeviceGet(ctypes.byref(dev), 0), "cuDeviceGet(0)")

# cuMemGetInfo — like module load, launch and memcpy — requires a CURRENT
# context: create it right after device selection, before any
# context-dependent call (bug hit on the first full run: the context used
# to be created AFTER cuMemGetInfo -> CUresult 201).
ctx = ctypes.c_void_p()
cu(cuda.cuDevicePrimaryCtxRetain(ctypes.byref(ctx), dev), "cuDevicePrimaryCtxRetain")
cu(cuda.cuCtxSetCurrent(ctx), "cuCtxSetCurrent")

name = ctypes.create_string_buffer(96)
cu(cuda.cuDeviceGetName(name, 96, dev), "cuDeviceGetName")

drv = ctypes.c_int()
cuda.cuDriverGetVersion(ctypes.byref(drv))  # best-effort
maj = ctypes.c_int()
mnr = ctypes.c_int()
cuda.cuDeviceGetAttribute(ctypes.byref(maj), 75, dev)  # 75 = COMPUTE_CAPABILITY_MAJOR
cuda.cuDeviceGetAttribute(ctypes.byref(mnr), 76, dev)  # 76 = COMPUTE_CAPABILITY_MINOR
free_b = ctypes.c_size_t()
total_b = ctypes.c_size_t()
cu(cuda.cuMemGetInfo_v2(ctypes.byref(free_b), ctypes.byref(total_b)), "cuMemGetInfo_v2")
print(f"  device 0      : {name.value.decode()}")
print(f"  compute cap   : sm_{maj.value}{mnr.value}  (driver {drv.value})")
print(f"  vram          : {total_b.value / 2**30:.2f} GiB total,"
      f" {free_b.value / 2**30:.2f} GiB free")

# ---- kernel: c[i] = a[i] + b[i], one block of N threads, hand-written PTX.
# .target sm_60 is forward-compatible: the driver JITs it to native (e.g. sm_120
# on Blackwell), which is precisely the libnvidia-ptxjitcompiler path we mount.
PTX = r"""
.version 7.0
.target sm_60
.address_size 64

.visible .entry addvec(
    .param .u64 pa,
    .param .u64 pb,
    .param .u64 pc
)
{
    .reg .b32 %r1;
    .reg .b64 %rd1, %rd2, %rd3, %rd4, %rd5;
    .reg .f32 %f1, %f2;

    mov.u32       %r1, %tid.x;
    mul.wide.u32  %rd1, %r1, 4;
    ld.param.u64  %rd2, [pa];
    ld.param.u64  %rd3, [pb];
    ld.param.u64  %rd4, [pc];

    add.u64       %rd5, %rd2, %rd1;
    ld.global.f32 %f1, [%rd5];
    add.u64       %rd5, %rd3, %rd1;
    ld.global.f32 %f2, [%rd5];
    add.f32       %f1, %f1, %f2;
    add.u64       %rd5, %rd4, %rd1;
    st.global.f32 [%rd5], %f1;

    ret;
}
"""

t0 = time.time()
mod = ctypes.c_void_p()
cu(cuda.cuModuleLoadData(ctypes.byref(mod), PTX.encode() + b"\x00"),
   "cuModuleLoadData (PTX JIT)")
jit_ms = 1000 * (time.time() - t0)

kern = ctypes.c_void_p()
cu(cuda.cuModuleGetFunction(ctypes.byref(kern), mod, b"addvec"), "cuModuleGetFunction")
print(f"\n== kernel JIT  : PTX(sm_60) -> sm_{maj.value}{mnr.value} native, {jit_ms:.0f} ms ==")

N = 256
A = (ctypes.c_float * N)(*[float(i) for i in range(N)])
B = (ctypes.c_float * N)(*[float(i) * 2.0 for i in range(N)])
C = (ctypes.c_float * N)()

dA = ctypes.c_uint64()
dB = ctypes.c_uint64()
dC = ctypes.c_uint64()
for d, tag in ((dA, "A"), (dB, "B"), (dC, "C")):
    cu(cuda.cuMemAlloc_v2(ctypes.byref(d), 4 * N), f"cuMemAlloc_v2({tag})")
cu(cuda.cuMemcpyHtoD_v2(dA, ctypes.cast(A, ctypes.c_void_p), 4 * N), "copy A->dev")
cu(cuda.cuMemcpyHtoD_v2(dB, ctypes.cast(B, ctypes.c_void_p), 4 * N), "copy B->dev")

params = (ctypes.c_void_p * 3)(
    ctypes.cast(ctypes.byref(dA), ctypes.c_void_p),
    ctypes.cast(ctypes.byref(dB), ctypes.c_void_p),
    ctypes.cast(ctypes.byref(dC), ctypes.c_void_p),
)
t0 = time.time()
cu(cuda.cuLaunchKernel(kern, 1, 1, 1, N, 1, 1, 0, None, params, None),
   "cuLaunchKernel")
cu(cuda.cuCtxSynchronize(), "cuCtxSynchronize")
run_ms = 1000 * (time.time() - t0)
cu(cuda.cuMemcpyDtoH_v2(ctypes.cast(C, ctypes.c_void_p), dC, 4 * N), "copy dev->C")

for d in (dA, dB, dC):
    cuda.cuMemFree_v2(d)

bad = [i for i in range(N) if C[i] != float(i) * 3.0]
print(f"\n== kernel run  : addvec, grid(1,1,1) x block(256,1,1), {run_ms:.2f} ms ==")
print(f"  c[0..4]      : {[round(C[i], 1) for i in range(5)]} ... c[{N-1}] = {C[N-1]}")
if bad:
    die(f"wrong result at {len(bad)}/{N} elements"
        f" (first: idx {bad[0]}, got {C[bad[0]]}, want {bad[0] * 3.0})")

print(f"""
========================================
  SMOKE TEST PASSED
  device : {name.value.decode()} (sm_{maj.value}{mnr.value})
  attach : toolkit-free (nodes + libcuda + JIT libs from host)
  math   : {N}/{N} correct — c = a + b computed on the GPU
========================================""")