# SotA evidence — the four gaps (probed 2026-11-05)

Method: each of `vision.md`'s four claimed gaps is restated, evidenced from
primary sources (normative specs, kernel lists, vendor documentation), and
split into **WERE** (state when the gap was identified) vs **ARE** (state at
last check, November 2026). Adjacent work that partially addresses a gap is
named explicitly — the *boundary* of that work is the contribution.

## Gap 1 — no vendor-neutral accelerator resource class in OCI/CDI

**WERE and ARE: open, without qualification.** The normative evidence is
the CDI specification itself (v1.1.0, current main) [1]: the `kind` field
"specifies a label which uniquely identifies the device **vendor**",
required to be DNS-subdomain-prefixed (`nvidia.com/gpu`,
`vendor.com/foo`). CDI's stated design principle is "vendor independence:
each vendor maintains their own specification files" [1]. The entire
version history (v0.3.0 → v1.1.1) added vendor-neutral *mechanisms* — mount
types, annotations, IntelRdt, netDevices — never a vendor-neutral
*resource class*. The OCI runtime spec has no accelerator concept beyond
device nodes and cgroup rules.

Ecosystem practice: NVIDIA emits `nvidia.com/gpu={index|UUID|all}` [2][3];
Docker's own CDI documentation models devices as `vendor1.com/device` [4].
Kubernetes DRA generalizes *allocation*, but per-vendor drivers still
publish the resource claims; nothing defines what a container should *read*
to consume an abstract accelerator.

**accelrun's answer:** the `ACCEL_*` env contract (what apps read),
`accel.local/gpu` (a vendor-neutral CDI kind — proof the schema can carry
the class), abstract selection, and TOML probes for arbitrary hardware.

## Gap 2 — container runtimes are VRAM-centric; no unified-memory story

**WERE and ARE: open.** On the flagship unified-memory platform there is
no accelerator story at all. Apple's own `container` tool (2025) confirms
GPU passthrough is unsupported [5], for structural reasons: Apple GPUs sit
behind no IOMMU, have no SR-IOV, and the only software path (userspace-API
forwarding via ParavirtualizedGraphics) is not a security boundary [5] —
the same applies to Docker Desktop's Linux VM. Measured consequence (Aug
2026): a 14B-class LLM streaming at 88 tok/s native on Metal collapses to
9 tok/s inside a container — silent CPU fallback [6]. MLX cannot run in a
Linux container at all; Metal has no Linux implementation [7][8]. The
workarounds that emerged (Venus/MoltenVK Vulkan remoting, Docker Model
Runner) are partial, performance-penalized, or LLM-specific — evidence of
demand, not of a resource model.

On the discrete side, NVIDIA's MPS v3 memory partitioning (CUDA 13.4+) is
explicitly a VRAM mechanism (MIG unsupported; dmem/misc backends), and its
own documentation concedes that managed/UVM allocations are "only tracked
by the memory controller in system memory" [9] — unified memory is left to
host memcg, which is precisely the composition accelrun makes first-class.

**accelrun's answer:** `unified_with_host` as a first-class field of the
resource model; the enforce-vs-hint asymmetry (cgroup cap on unified
backends, budget hint for VRAM); a Metal probe that models the UMA platform
in the abstract claim even where device plumbing is structurally absent.

## Gap 3 — no cgroup controller for "GPU memory" inside unified RAM

**WERE: open without qualification.** Proposals existed and stalled: the
DRM cgroup RFC (2021) [10] and Google's `gpu` cgroup controller RFC v2/v3
(Feb–Mar 2022: `gpu.memory.current` / `gpu.memory.max`) [11] were never
merged; the 2022 RFC itself argued memcg was a poor fit for device buffers
(page-based vs buffer-based accounting) [11].

**ARE: partially closed — and the boundary matters.** Linux 6.14 (merged
January 2025) gained the **dmem** controller for *device* memory:
per-device accounting and limits (`dmem.max`), integrated through
DRM/TTM, initial driver support in Intel Xe, AMD patches outstanding
[12][13]. NVIDIA consumes it (CUDA 13.4+ MPS v3) with a driver-internal
fallback [9]. But dmem covers device-attached memory — **VRAM** — not
unified RAM. "GPU memory inside unified RAM" (Apple, APUs, CPU-class
acceleration, UVM/managed allocations) still lives under the plain memory
controller with no device attribution — which is why accelrun's
composition (cgroup v2 `memory.max` on unified backends) remains the only
working mechanism for that half. The upstream motion validates the need;
the unified half of the gap is the open research question.

## Gap 4 — no fault model / RAMS in portable-GPU tooling

**WERE and ARE: open.** The CDI specification's entire error-handling
section is four user-facing conditions ("surface an error when...") [1] —
no fault registry, no detection semantics. Kubernetes (SIG Node, July
2025): the scheduler's "view on resources is still very static... lacks
good support for handling full or partial hardware failures"; "there is
very little handling of device failure in Kubernetes today"; the first
roadmap step (KEP 4680) is merely to *report* device health in pod status;
everything beyond that is DIY (health controllers, pod watchers) [14].

The failures are first-order, not hypothetical: Llama 3 405B training saw
419 unexpected interruptions in 54 days (~1 per 3 hours), GPU/HBM the
largest share [15]; NVIDIA's GeForce NOW reports ~19 remediation requests
per 1000 nodes per day [14].

Adjacent work exists at *other* layers — which sharpens, rather than closes,
the gap: Dynamo ships XID fault injection for inference *serving* [16];
BitIR (2026) injects IR-level faults across NVIDIA/Intel/AMD on three
leadership supercomputers and finds resilience is "not backend-invariant"
— the same fault masks on one backend, silently corrupts on another, fails
hard on a third [17]; FMEA appears in AI *facilities* engineering and agent
tooling — but none of this is a fault model for the container
accelerator-attach layer, and none of it ships inside a launcher.

**accelrun's answer:** a five-fault FMEA registry with a live detection
hook (ProbeFalseNegative), and a runbook that is an operator-facing failure
taxonomy distilled from real incidents. The planned fault-injection study
turns the model into validated RAMS — with BitIR as the methodological
precedent that backend-aware fault analysis is a publishable finding.

## Summary

| Gap | WERE | ARE (2026-11) | accelrun's mechanism |
|---|---|---|---|
| 1 — vendor-neutral class | open: CDI kinds are vendor-scoped | open: spec unchanged; DRA still per-vendor drivers | `ACCEL_*` + `accel.local/gpu` |
| 2 — VRAM-centric, no UMA | open: no GPU in Apple containers at all | open: workarounds partial; NVIDIA partitions VRAM only, UVM left to memcg | `unified_with_host`, enforce-vs-hint |
| 3 — GPU-mem cgroup | open: RFCs stalled (2021–22) | half-closed: dmem (6.14) covers device VRAM only; unified RAM still plain memcg | `memory.max` composition on unified |
| 4 — fault model | open: specs silent | open: CDI = 4 error conditions; k8s = DIY + health *reporting* only | FMEA registry + runbook |

Implication for the position paper: the gap-3 trajectory (RFC 2022 →
merged dmem 2025, for *device* memory only) is itself evidence that the
needs are real and recognized upstream, while the unified-memory half —
the one accelrun composes today — remains the unclaimed territory.

## Sources (accessed 2026-11-05)

[1] CDI Specification v1.1.0 — github.com/cncf-tags/container-device-interface/blob/main/SPEC.md
[2] NVIDIA Container Toolkit, CDI Support — docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/cdi-support.html
[3] nvidia-ctk README — github.com/NVIDIA/nvidia-container-toolkit/blob/main/cmd/nvidia-ctk/README.md
[4] Docker Docs, CDI — docs.docker.com/build/building/cdi/
[5] apple/container discussion #62: GPU passthrough — github.com/apple/container/discussions/62
[6] Contra Collective, "The GPU Passthrough Problem" (Aug 2026) — contracollective.com/blog/local-llm-inference-docker-apple-silicon-gpu-passthrough-2026
[7] MLX issue #1945 — github.com/ml-explore/mlx/issues/1945
[8] MLX discussion #3349 (docker_mlx_cpp) — github.com/ml-explore/mlx/discussions/3349
[9] NVIDIA MPS v3 Memory Partitioning — docs.nvidia.com/deploy/mps/latest/mpsv3-memory-partitioning.html
[10] DRM cgroup RFC (2021) — lore.kernel.org/amd-gfx/20210126214626.16260-1-brian.welty@intel.com/
[11] GPU cgroup controller RFC v3 (Mar 2022) — lists.freedesktop.org/archives/dri-devel/2022-March/346104.html
[12] dmem cgroup PR (Jan 2025, Linux 6.14) — lists.freedesktop.org/archives/dri-devel/2025-January/485282.html
[13] Phoronix, "DMEM cgroup vRAM Control" (Jan 2025) — phoronix.com/news/DMEM-cgroup-vRAM-Control
[14] Kubernetes blog, "Navigating Failures in Pods With Devices" (Jul 2025) — kubernetes.io/blog/2025/07/03/navigating-failures-in-pods-with-devices/
[15] Meta, "The Llama 3 Herd of Models" (arXiv 2407.21783), §3.3.4
[16] NVIDIA Dynamo, Fault Tolerance Testing — docs.nvidia.com/dynamo/dev/knowledge-base/concepts/fault-tolerance/fault-tolerance-testing
[17] BitIR (arXiv 2610.04037) — arxiv.org/abs/2610.04037
