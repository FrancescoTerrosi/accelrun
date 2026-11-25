//! FMEA-style fault registry with detection hooks.
//!
//! History: this file also wrote a cgroup-v2 memory cap for unified backends
//! — removed 2026-11. It moved the *accelrun CLI's own PID* into a cgroup,
//! but containers are placed in cgroups by dockerd, so the cap never reached
//! the workload (and its "capped unified RAM" success message was fiction).
//! The real enforcement lever is the launcher-issued `docker --memory`
//! (oci.rs), applied in BOTH direct and `--cdi` modes and landing in the
//! container's cgroup v2 `memory.max`. CDI has no memory-limit field, so
//! the emitted spec carries no cgroup rule (see cdi.rs). See
//! docs/decisions.md, model-conformance fixes.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    ProbeFalseNegative,
    UnifiedHostOOM,
    VramOvercommit,
    BackendCrashMidRun,
    NumericDrift,
}

impl Fault {
    pub fn description(&self) -> &'static str {
        match self {
            Fault::ProbeFalseNegative => "GPU present but not detected -> silently ran on CPU",
            Fault::UnifiedHostOOM => "unified-RAM container OOMed the whole host, not just itself",
            Fault::VramOvercommit => "consumer NVIDIA has no hard VRAM cap; app exceeded VRAM",
            Fault::BackendCrashMidRun => "kernel panic / device reset mid-run",
            Fault::NumericDrift => "same graph, different backend -> results outside tolerance",
        }
    }

    pub fn mitigation(&self) -> &'static str {
        match self {
            Fault::ProbeFalseNegative => "re-run `accelrun probe`; check device nodes",
            Fault::UnifiedHostOOM => "lower --mem below total RAM (docker --memory caps the container)",
            Fault::VramOvercommit => "use ACCEL_MEM_BYTES app-level budget; quantize model",
            Fault::BackendCrashMidRun => "check dmesg; re-run with ACCEL_BACKEND=cpu fallback",
            Fault::NumericDrift => "compare output hashes across backends; set tolerance in test",
        }
    }
}

/// Cumulative OOM kills since boot (kernel counter, /proc/vmstat `oom_kill`).
/// Host-level evidence for the UnifiedHostOOM fault: a unified-RAM container
/// that OOMs the whole host increments this. (The old hook read the
/// `memory.events` of a cgroup the CLI had created for itself — never the
/// container's — so it could only ever read 0.)
pub fn host_oom_kills() -> u64 {
    std::fs::read_to_string("/proc/vmstat")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("oom_kill "))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0)
}

pub fn detect(f: Fault, ctx_backend: &str) -> bool {
    match f {
        Fault::UnifiedHostOOM => host_oom_kills() > 0,
        Fault::ProbeFalseNegative => {
            // "GPU present but not detected -> silently ran on CPU": only
            // meaningful when selection actually landed on CPU (any other
            // backend means the accelerator WAS detected). Presence sources
            // mirror the probes: `nvidia-smi -L` (works even without numbered
            // device nodes — the WSL case) and `/dev/kfd` (the AMD probe's
            // gate). The pre-fix hook checked nvidia-smi only, so an AMD
            // probe miss was structurally invisible. Custom TOML probes have
            // no generic presence signal — uncovered, documented gap.
            if ctx_backend != "cpu" {
                return false;
            }
            let nvidia_listed = Command::new("nvidia-smi")
                .arg("-L")
                .output()
                .map(|o| {
                    o.status.success()
                        && String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains("GPU"))
                })
                .unwrap_or(false);
            nvidia_listed || Path::new("/dev/kfd").exists()
        }
        _ => false,
    }
}
