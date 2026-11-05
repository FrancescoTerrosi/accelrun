//! cgroups.rs and fault.rs merged: cgroup v2 memory cap for unified backends
//! plus the FMEA-style fault registry.

use std::fs;

pub fn apply_memory_cap(tag: &str, bytes: u64) -> std::io::Result<()> {
    let dir = format!("/sys/fs/cgroup/accelrun/{}", tag);
    fs::create_dir_all(&dir)?;
    fs::write(format!("{}/memory.max", dir), bytes.to_string())?;
    fs::write(format!("{}/cgroup.procs", dir), std::process::id().to_string())?;
    Ok(())
}

/// Read OOM events — used by the fault layer to detect UnifiedHostOOM.
pub fn oom_count(tag: &str) -> u64 {
    fs::read_to_string(format!("/sys/fs/cgroup/accelrun/{}/memory.events", tag))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("oom "))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0)
}

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
            Fault::UnifiedHostOOM => "lower --mem below total RAM; keep cgroup memory.max set",
            Fault::VramOvercommit => "use ACCEL_MEM_BYTES app-level budget; quantize model",
            Fault::BackendCrashMidRun => "check dmesg; re-run with ACCEL_BACKEND=cpu fallback",
            Fault::NumericDrift => "compare output hashes across backends; set tolerance in test",
        }
    }
}

pub fn detect(f: Fault, ctx_backend: &str) -> bool {
    match f {
        Fault::UnifiedHostOOM => oom_count("default") > 0,
        Fault::ProbeFalseNegative => {
            // Only flag when nvidia-smi lists GPUs but the *selected* backend
            // is NOT nvidia and no device nodes exist. If the smi-fallback
            // already selected nvidia, that is a correct detection.
            if ctx_backend == "nvidia" {
                return false;
            }
            let listed = std::process::Command::new("nvidia-smi")
                .arg("-L")
                .output()
                .map(|o| {
                    o.status.success()
                        && String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains("GPU"))
                })
                .unwrap_or(false);
            listed && crate::probe::glob_dev("/dev/nvidia[0-9]*").is_empty()
        }
        _ => false,
    }
}
