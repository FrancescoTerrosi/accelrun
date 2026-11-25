# CDI attach hint — plan

> **Status: implemented (2026-11).** `cargo test` 11/11 and
> `cargo build --release` pass; `--cdi --dry-run` is side-effect-free; the
> non-root `--cdi` path errors cleanly. Only step 4 (sudo install + sidecar
> write on a writable daemon dir) remains to be exercised on a root-capable
> host. The hint is *documentation*, not a claim that `--cdi` runs compute.

## 1. Why

`--cdi` publishes a vendor-neutral `accel.local/gpu` spec (the `ACCEL_*`
contract + device nodes) but deliberately omits the backend-specific
driver-library mounts that direct mode assembles. The hint makes accelrun
*state* those mounts instead of leaving a consumer to reverse-engineer them.

## 2. Context / decision

- `--cdi` stays **publication/demonstration only**; direct mode remains the
  compute-attach path.
- The CDI spec stays `env` + `deviceNodes`; it does not grow a `mounts`
  field. (That is the deeper end-state, out of scope here.)
- accelrun already computes the mount set per backend in `oci.rs`; this plan
  factors it out and reuses it for the hint.

## 3. Changes

### 3.1 Factor the mount computation out of `oci.rs`

- Introduce a backend-generic helper, e.g.
  `attach_mounts(acc: &Accelerator) -> Vec<(String, String)>`
  returning `(host_path, container_path)` pairs.
- Move the NVIDIA and AMD mount logic (driver libs, `nvidia-smi` /
  `rocminfo`/`amd-smi`/`rocm-smi`, multiarch mirror, merge-usr dedup) into it.
- Direct mode keeps consuming the same list as `-v` args — **no argv change**.
- CPU and custom backends return an empty list (consistent with today).

### 3.2 Emit the hint in `--cdi` mode

- On `--cdi` (non-dry-run), after installing the spec, print a human-readable
  hint, e.g.:

  ```
  [accelrun] mode=cdi: backend=nvidia — vendor-neutral publication only
  [accelrun] attach hint (mounts a consumer must add for compute):
    /usr/lib64/libcuda.so.1            -> /usr/lib/x86_64-linux-gnu/libcuda.so.1
    /usr/lib64/libnvidia-ml.so.1       -> /usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1
    ... (PTX JIT / compiler libs, /usr/bin/nvidia-smi)
  ```

- Write the same list as a machine-readable sidecar:
  `/tmp/accelrun-cdi/mount-hints.txt` (one `host<TAB>container` pair per line),
  so an orchestrator can consume it without parsing stdout.
- `--dry-run --cdi` prints the hint and previews the install, but performs
  neither (no side effects) — consistent with the dry-run contract.

### 3.3 Cleanup parity

`clean.sh` already removes what `--cdi` installs:

- `/tmp/accelrun-cdi` (the per-run spec + any sidecar we add), and
- any `accel.local/gpu` spec found by content in `/var/run/cdi` and `/etc/cdi`.

Two gaps to close in `clean.sh` for completeness:

1. **Daemon reload after removal** — the daemon deregisters a removed spec only
   on reload. Add an explicit note (and/or an optional flag) so `clean.sh` says
   this, matching the install-side "may need a reload" wording.
2. **Rootless Docker CDI dirs** — rootless dockerd can be configured with
   custom `cdi-spec-dirs`. `clean.sh` currently only scans the two defaults.
   Either detect them via `docker info` or document the limitation.

## 4. Explicitly NOT doing

- **Not** putting `mounts` into the CDI spec (the deeper end-state).
- **Not** making `--cdi` a working compute-attach path.
- **Not** auto-reloading the daemon on install or removal.
- **Not** changing the spec's `kind`, `cdiVersion`, or the direct-mode argv.

## 5. Verification (each step must pass once in its life)

1. `cargo test` — existing suite plus any new unit tests for `attach_mounts`.
2. `./accelrun run --mem 12Gi --image X --dry-run -- true` (direct) — argv is
   byte-identical to before the refactor.
3. `./accelrun run --mem 12Gi --image X --cdi --dry-run -- true` — prints the
   hint, previews the install, does not create `/var/run/cdi/accel.json`.
4. `sudo ./accelrun run --mem 12Gi --image X --cdi -- true` — installs the
   spec, prints the hint; `cat /tmp/accelrun-cdi/mount-hints.txt` matches the
   direct-mode `-v` mounts for the same backend.
5. `./clean.sh --dry-run` — inventory now mentions the mount-hints sidecar and
   the reload note.

## 6. Honesty rules

- The hint is **documentation of what direct mode does**, not enforcement and
  not a promise that `--cdi` runs compute.
- No README/runbook wording may imply `--cdi` became a launch path as a result
  of this change.
