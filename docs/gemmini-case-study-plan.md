# Gemmini case study — plan

> **Status: DRAFT — proposed, not yet executed.** This file is the plan.
> Results land only after the verification ladder passes: session entry in
> `decisions.md`, an ARE-update in `sota-gaps.md`, README verified-status
> update, and (only then) the paper claims. Same rules as the rest of the
> repo: AI-assisted drafting, human-verified execution, and *no claim
> without a PASS on hardware*.

Target hardware: [Gemmini](https://github.com/ucb-bar/gemmini) (UC Berkeley,
DAC'21) — the Chipyard RoCC systolic-array DNN accelerator generator — as the
first non-NVIDIA, non-CPU **custom-accelerator** verification of accelrun.

## 1. Why: which weak claims this attacks

| Current claim | Current evidence | The hole a reviewer sees |
|---|---|---|
| "Register custom accelerators without recompiling" | 77 LOC (`src/probe/custom.rs`) + example TOML | never validated against real exotic hardware |
| `unified_with_host` → cgroup v2 `memory.max` **enforcement** | code paths (`src/sys.rs`, `cdi.rs`, docker `--memory`) | the enforce side has **never been exercised on any accelerator host**: the only verified backend (NVIDIA) is hint-only (`unified=false`), and every recorded cgroup write failed (no root) |
| Vendor-neutral resource class (`sota-gaps.md` Gap 1) | document analysis | no demonstration with a non-NVIDIA accelerator |

Why Gemmini specifically:

- **Canonical academic artifact.** Citing and building on the Berkeley
  generator is stronger related-work posture than an anonymous custom board.
- **It is literally the `unified_with_host` case.** Scratchpad + DMA from
  shared SoC DRAM (through L2): no hardware memory boundary → the budget is
  enforceable, not hinted. This would be the **first end-to-end exercise of
  the enforce-vs-hint asymmetry** — i.e. a live demonstration of Gap 2/3's
  "unified half", which the position paper currently supports with document
  analysis only.
- **It exposes a real boundary of the abstraction.** Mainstream Gemmini is
  RoCC/ISA-attached — there is **no `/dev` node**, so the current custom
  probe cannot even detect it. Finding, patching, and documenting that
  boundary is exactly the bug-driven evolution this repo runs on (cf. the
  five-bug chain).

## 2. Claims to substantiate (and claims NOT made)

- **C1 — extension mechanism**: a `[[probe]]` entry for non-builtin
  hardware yields correct probe → select → inject → contract, no recompile.
- **C2 — unified enforcement**: on a unified accelerator host, `--mem` is
  *enforced* — a container that exceeds `ACCEL_MEM_BYTES` is OOM-killed.
  First verification of the enforce side of the memory model.
- **C3 — contract neutrality**: a non-NVIDIA workload consumes the same
  `ACCEL_*` contract inside an accelrun-launched container.
- **C4 — vendor-neutral publication**: the resource is emitted as CDI
  `accel.local/gpu=gpu0` (env + deviceNodes) — schedulable next to NVIDIA.
  Memory enforcement rides the same `docker --memory` as direct mode, not
  the spec: CDI has no memory-limit field.

Explicitly NOT claimed (write these down or reviewers will):

- **Not** cross-backend numeric parity (same model on NVIDIA vs Gemmini) —
  that is the separate numeric-drift study; phase-2 optional via Gemmini's
  ONNX-Runtime RISC-V port.
- **Not** a replacement for the AMD ROCm second-backend item (TODO, medium
  term). AMD validates a builtin probe and the conventional cross-vendor
  story; Gemmini validates the *extension mechanism* and *unified
  enforcement*. Sequencing if forced: AMD is cheaper and broader; Gemmini is
  higher-novelty for the paper (Gaps 1–3). Both belong in the evidence base.
- **Not** "attach" for ISA-attached accelerators: containers pass the CPU
  ISA through by construction — RoCC instructions execute identically in
  any container on the SoC. accelrun's value there is **contract + cap +
  CDI publication**, not device injection. The write-up must say this
  plainly; claiming device attach for a nodeless accelerator would be
  inflated.

## 3. Tiers

### Tier 0 — no Gemmini hardware (do first; ~1 day)

Validate the entire accelrun-side machinery for the `custom` backend with a
synthetic device node. Repeatable script: `tests/custom-probe-smoke.sh`.

```bash
# unbound dynamic-major char node: does not need a real driver behind it
# (docker mknod's it into the container; opening it gives ENXIO, which is
# fine — Tier 0 verifies probe/select/inject/contract, not compute)
sudo mknod /dev/fakeaccel c 511 0

mkdir -p ~/.config/accelrun && cat > ~/.config/accelrun/probes.toml <<'EOF'
[[probe]]
id = "gemmini-sim"
device_glob = "/dev/fakeaccel"
mem_gib = 2
unified = true
EOF

./accelrun probe                                   # custom listed, ranked above cpu
./accelrun run --mem 1Gi --image python:3.12-slim --dry-run -- true
#   expect: --device /dev/fakeaccel, -e ACCEL_BACKEND=custom, ACCEL_UNIFIED=true
./accelrun run --mem 1Gi --image python:3.12-slim -- \
    python3 -c "import os; print(os.environ['ACCEL_BACKEND'], os.environ['ACCEL_MEM_BYTES'])"
docker inspect <c> --format '{{json .HostConfig.Devices}}'
cat /tmp/accelrun-cdi/accel.json                   # env + deviceNodes only (no memory rule)
```

What Tier 0 already catches: TOML-subset parser bugs, glob-class behavior
(the five-bug chain started exactly there), `find_map` ordering, unified
plumbing into env + CDI, and ranking (custom 500 > cpu 50). The 2026-11
model-conformance session fixed the backend gating this plan previously
flagged: NVIDIA libs/env are now injected only on the nvidia backend — a
custom probe gets nodes + group-add only, nothing vendor-specific (see
`oci.rs` and decisions.md).

**Honesty rule:** Tier 0 is *contract verification*, not "verified on
custom hardware". The README wording must reflect that.

### Tier 1 — code changes the study forces (~2 days, no hardware needed to write)

- **P0 — fix the fallback warning** (`src/main.rs`): DONE (2026-11
  model-conformance session) — `select` now tiers accelerators-meeting →
  accelerators-short → CPU and the warning names the degraded backend
  and the budget gap; unit-tested in `select.rs`. No action left for this
  study.
- **P1 — nodeless (env-only) custom probes** (`src/probe/custom.rs`,
  ~30 LOC): allow detection via a `detect_glob` (e.g. a sysfs path) with no
  `device_glob` → `devices: []`. The launcher already handles this shape
  (Apple/Metal precedent, `src/probe/apple.rs`; `oci.rs` injects env-only
  when `devices` is empty). TOML:
  ```toml
  [[probe]]
  id = "gemmini"
  detect_glob = "/sys/bus/platform/devices/gemmini"   # presence only — NOT injected
  mem_gib = 1
  unified = true
  ```
- **P2 — image-borne userspace stack (document, don't patch)**: Gemmini
  software is *config-generated* (`gemmini_params.h` is elaborated per-SoC),
  so there is no host "driver userspace" library set to bind-mount — the
  NVIDIA lib-mount pattern does not generalize. Decision: the Gemmini stack
  (gemmini.h / libgemmini / ONNX-Runtime RISC-V) rides **inside the image**.
  Record this boundary in the runbook — it is a finding, not a TODO.
- **P3 — smoke tests** (`tests/gemmini-smoke.c`): minimal matmul via
  gemmini.h (mvin A,B → matmul → mvout C → memcmp) printing the `ACCEL_*`
  contract, plus an enforcement section: allocate past `ACCEL_MEM_BYTES` →
  expect the kernel OOM-kill → print `ENFORCED`. Note the OOM test works
  through docker's `--memory` (set by `oci.rs` for unified backends — the
  Gemmini TOML declares `unified = true`, so the flag is present); it needs
  no root and does not depend on the removed CLI-side cgroup writer.

All patches stay zero-dependency; no RISC-V toolchain enters the repo.

### Tier 2 — real Gemmini host (gated on hardware access)

| Venue | What runs | Assessment |
|---|---|---|
| **FireSim (AWS EC2 F1)** | full Linux on the simulated Gemmini SoC; Docker inside the simulated node | preferred for the paper: rentable, scriptable, repeatable. Slow (sim node at FPGA-class MHz; one-time rootfs build via FireMarshal incl. the docker daemon) — **timebox the setup** |
| **Chipyard FPGA board** (VCU118-class) | same SoC on local hardware | fastest path if a board is at hand |
| **gemmini-device-driver + GemminiOS** (research stack) | `/dev/gemmini` on the RISC-V Linux → the *existing* TOML probe works unmodified | validates the zero-code-change claim (C1) in its strongest form, but the stack is research-grade (static into a 5.5-era kernel) |
| spike / Verilator / QEMU | — | **excluded**: no full Linux (spike pk, baremetal) or no RoCC model (QEMU). Document why, to preempt "why not just simulate" |

Verification ladder (runbook-style, one PASS each):

1. **Host baseline**: run `gemmini-smoke.c` natively on the SoC — the
   "same snippet, host vs container" bisection rule.
2. `accelrun probe` (after P1): `custom gemmini` listed alongside `cpu`.
3. `--dry-run`: env-only argv — **no** `--device` (correct for nodeless).
4. `docker inspect`: `Devices: []`, `Memory == ACCEL_MEM_BYTES`.
5. In-container: `cat /sys/fs/cgroup/memory.max` == `ACCEL_MEM_BYTES`.
6. In-container: `gemmini-smoke.c` matmul PASS (compute really happened —
   record the systolic-array path, not just "didn't crash").
7. In-container: OOM-kill at budget → `ENFORCED` (C2, first time ever).
8. CDI spec inspection: env + deviceNodes present; no memory rule (C4).

Success criteria: every step passes **at least once in its life** (the
five-bug-chain lesson); every bug found becomes a numbered entry in the
`decisions.md` chain, no exceptions.

### Tier 3 — opportunistic: driver-node variant

If the `gemmini-device-driver` research stack is built, register it with the
**unmodified** custom probe:

```toml
[[probe]]
id = "gemmini"
device_glob = "/dev/gemmini"
mem_gib = 1
unified = true
```

and run the Tier-2 ladder including `--device` injection. This is the
strongest form of C1: exotic hardware registered with zero code change.

## 4. Artifacts that land in the repo

- `tests/custom-probe-smoke.sh` (Tier 0) and `tests/gemmini-smoke.c` (Tier 2)
- `docs/decisions.md`: session entry (bug chain if one surfaces)
- `docs/runbook.md`: "Custom accelerators" section + the per-backend ladder
- `README.md`: verified-status table — keep the honest split: NVIDIA
  (end-to-end), custom+unified-cap (after Tier 2), Tier 0 = contract-only
- `docs/sota-gaps.md`: ARE-update — Gaps 1–3 gain a *demonstration*, not
  just document analysis
- close the medium-term case-study item (the old `TODO.md` tracker is gone;
  items now live in the session queue / decisions log)

## 5. Risks and honest caveats

- **Hardware access gates Tier 2.** Mitigation: Tier 0 + Tier 1 land value
  with zero hardware; Tier 2 is a rentable FireSim exercise.
- **Overclaiming is the biggest reputational risk.** The project's
  credibility rests on the WERE/ARE discipline; a Tier-0-only result must
  never be worded as "verified on custom hardware".
- **Scope creep**: no RISC-V toolchain, no QEMU plugins, no multi-select,
  no new deps in the launcher. Single-node scope unchanged.
- **The ISA-attach nuance** (Section 2) must be explicit, or the study
  reads as inflating what accelrun does.

## 6. Sequencing

Decision (2026-11): **AMD ROCm end-to-end goes first for hardware-gated
verification; this study's zero-hardware tiers land now.** Rationale: AMD
substantiates the headline portability thesis (a reviewer asks "second
vendor?" before "extensible?"), and its code path carried a visible latent
bug chain — the 2026-11 code session has since fixed those blind
(kfd-injection, APU heuristic, ROCm userspace bind-mounts, `--group-add`;
see `docs/decisions.md`), and settled the userspace question the same way
this plan's P2 needs: host bind-mounts for the AMD *builtin* (mirroring
NVIDIA), image-borne stacks for *custom* accelerators. What the hardware
session still owns: the first PASS and whatever chain it surfaces.

1. **Now, no hardware**: Tier 0 + P0/P1 patches — the userspace-stack
   question is largely pre-answered by the AMD session's decision above.
2. **Hardware-gated, first**: AMD ROCm end-to-end (attach path implemented,
   argv-verified; `tests/amd-smoke.py` exists for the ladder — never yet
   run on hardware).
3. **Hardware-gated, second**: FireSim timebox (1–2 weeks calendar, mostly
   setup); fall back to a board if F1 friction exceeds it.
4. Write-up: decisions entry → runbook → README → sota-gaps ARE-update.
5. Phase 2 (optional): same ONNX model on NVIDIA and Gemmini via the
   ONNX-Runtime RISC-V port — feeds the numeric-drift study.

Override rule: if AMD hardware is unobtainable and FireSim is easy, swap
steps 2 and 3 — whichever end-to-end pass is *reachable* beats the one that
is merely sequenced.
