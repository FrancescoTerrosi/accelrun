//! NVIDIA probe: /dev/nvidia* nodes + nvidia-smi query. No NVML linkage.

use super::*;
use std::process::Command;

pub struct NvidiaProbe;

impl Probe for NvidiaProbe {
    fn detect(&self) -> Option<Accelerator> {
        let gpus = glob_dev("/dev/nvidia[0-9]*");
        // Fallback: driver present (nvidia-smi lists a GPU) but no device
        // nodes visible (WSL, wrong namespace) — still detectable.
        let smi_lists_gpu = Command::new("nvidia-smi")
            .arg("-L")
            .output()
            .map(|o| {
                o.status.success()
                    && String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains("GPU"))
            })
            .unwrap_or(false);
        if gpus.is_empty() && !smi_lists_gpu {
            return None;
        }
        // Device list: numbered nodes + control nodes when present
        // (control nodes are needed for toolkit-free attach).
        // Control nodes ALONE are useless: NVML needs the numbered
        // /dev/nvidia[0-9]* nodes to enumerate devices. On hosts where the
        // driver works (smi fallback) but no numbered nodes are visible
        // (WSL-like), keep devices EMPTY so launch() takes the --gpus all
        // last resort instead of a broken raw-node injection
        // ("No devices were found" inside the container).
        let mut devices = gpus;
        if !devices.is_empty() {
            for extra in ["/dev/nvidiactl", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools"] {
                if std::path::Path::new(extra).exists() {
                    devices.push(extra.to_string());
                }
            }
        }
        let out = Command::new("nvidia-smi")
            .args([
                "--query-gpu=name,memory.total",
                "--format=csv,noheader,nounits",
            ])
            .output()
            .ok()?;
        let line = String::from_utf8_lossy(&out.stdout);
        let first = line.lines().next()?;
        let mut it = first.split(',').map(|s| s.trim());
        let name = it.next()?.to_string();
        let mem_mib: u64 = it.next()?.parse().ok()?;
        Some(Accelerator {
            backend: Backend::Nvidia,
            name,
            mem_total: mem_mib * 1024 * 1024,
            // HBM-class consumer card; exact value not critical for ranking.
            mem_bandwidth_gbps: 1000.0,
            devices,
            unified_with_host: false,
            custom_accel_id: None,
        })
    }
}

/// Human-readable diagnostics for `accelrun probe`.
pub fn diagnostics() -> Vec<String> {
    let nodes = glob_dev("/dev/nvidia[0-9]*");
    let smi = Command::new("nvidia-smi")
        .arg("-L")
        .output()
        .map(|o| {
            if o.status.success() {
                let n = String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("GPU")).count();
                format!("nvidia-smi -L: OK, {} GPU(s)", n)
            } else {
                format!("nvidia-smi -L: failed ({})", String::from_utf8_lossy(&o.stderr).trim())
            }
        })
        .unwrap_or_else(|e| format!("nvidia-smi not found ({})", e));
    vec![
        format!("/dev/nvidia[0-9]* nodes: {:?}", nodes),
        smi,
        if nodes.is_empty() {
            "note: no device nodes visible -> will use --gpus all if driver present".to_string()
        } else {
            String::new()
        },
    ]
}
