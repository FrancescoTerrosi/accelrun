//! Apple Metal probe: unified LPDDR on Apple Silicon. Under Docker Desktop the
//! Linux VM exposes Metal via the paravirtual GPU stack, so we only set env
//! and let the runtime handle device plumbing.

use super::*;
use std::process::Command;

pub struct AppleProbe;

impl Probe for AppleProbe {
    fn detect(&self) -> Option<Accelerator> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let model = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()?;
        let brand = String::from_utf8_lossy(&model.stdout);
        if !brand.contains("Apple") {
            return None;
        }
        let mem = Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())?;
        Some(Accelerator {
            backend: Backend::Metal,
            name: brand.trim().to_string(),
            mem_total: mem, // unified: whole pool, capped later by cgroups
            mem_bandwidth_gbps: 800.0, // M-series UMA class
            devices: vec![],
            unified_with_host: true,
            custom_accel_id: None,
        })
    }
}
