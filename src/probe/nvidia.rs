//! NVIDIA probe: /dev/nvidia* nodes + nvidia-smi query. No NVML linkage.
//! WSL2 hosts: /dev/dxg node + the Windows driver's /usr/lib/wsl/lib.

use super::*;
use std::path::Path;
use std::process::Command;

pub struct NvidiaProbe;

/// Run nvidia-smi, trying WSL's driver-mounted copy first: on WSL2 the
/// Windows driver ships nvidia-smi (and libcuda, ...) under
/// /usr/lib/wsl/lib, which non-Ubuntu distros often leave off PATH — a
/// PATH-only lookup silently degrades WSL hosts to the CPU backend.
fn smi(args: &[&str]) -> Option<std::process::Output> {
    for exe in ["/usr/lib/wsl/lib/nvidia-smi", "nvidia-smi"] {
        if let Ok(out) = Command::new(exe).args(args).output() {
            if out.status.success() {
                return Some(out);
            }
        }
    }
    None
}

impl Probe for NvidiaProbe {
    fn detect(&self) -> Option<Accelerator> {
        let gpus = glob_dev("/dev/nvidia[0-9]*");
        // WSL2 paravirt GPU: numbered /dev/nvidia* nodes NEVER exist there —
        // /dev/dxg is the only device node. A dxg alone proves nothing (it
        // is the vendor-neutral DirectCompute interface); the smi check
        // below must still confirm an NVIDIA GPU.
        let wsl_gpu = Path::new("/dev/dxg").exists();
        // Fallback: driver present (nvidia-smi lists a GPU) but no device
        // nodes visible (WSL, wrong namespace) — still detectable.
        let smi_lists_gpu = smi(&["-L"])
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains("GPU")))
            .unwrap_or(false);
        if gpus.is_empty() && !smi_lists_gpu {
            return None;
        }
        // Device list: numbered nodes + control nodes when present
        // (control nodes are needed for toolkit-free attach).
        // Control nodes ALONE are useless: NVML needs the numbered
        // /dev/nvidia[0-9]* nodes to enumerate devices. On hosts where the
        // driver works (smi fallback) but no numbered nodes are visible:
        //  - WSL2 (dxg present): attach the dxg node — launch() mounts
        //    /usr/lib/wsl/lib + LD_LIBRARY_PATH, the same shape as
        //    nvidia-container-toolkit's own WSL mode, still toolkit-free.
        //  - anything else (restricted namespace): keep devices EMPTY so
        //    launch() takes the --gpus all last resort instead of a broken
        //    raw-node injection ("No devices were found" in-container).
        let mut devices = gpus;
        if !devices.is_empty() {
            for extra in ["/dev/nvidiactl", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools"] {
                if Path::new(extra).exists() {
                    devices.push(extra.to_string());
                }
            }
        } else if wsl_gpu && smi_lists_gpu {
            devices.push("/dev/dxg".to_string());
        }
        let out = smi(&[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])?;
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
    let smi_line = match smi(&["-L"]) {
        Some(o) => {
            let n = String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("GPU")).count();
            format!("nvidia-smi -L: OK, {} GPU(s)", n)
        }
        None => "nvidia-smi -L: not found or failed (tried PATH and /usr/lib/wsl/lib)".to_string(),
    };
    let mut out = vec![
        format!("/dev/nvidia[0-9]* nodes: {:?}", nodes),
        smi_line,
    ];
    if Path::new("/dev/dxg").exists() {
        out.push("/dev/dxg: present (WSL2) -> toolkit-free WSL attach".to_string());
    } else if nodes.is_empty() {
        out.push(
            "note: no device nodes visible -> will use --gpus all if driver present".to_string(),
        );
    }
    out
}
