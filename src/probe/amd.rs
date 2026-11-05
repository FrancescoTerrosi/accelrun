//! AMD probe: /dev/kfd present + rocm-smi.

use super::*;
use std::process::Command;

pub struct AmdProbe;

impl Probe for AmdProbe {
    fn detect(&self) -> Option<Accelerator> {
        if !std::path::Path::new("/dev/kfd").exists() {
            return None;
        }
        let out = Command::new("rocm-smi")
            .args(["--showmeminfo", "vram", "--csv"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mem_total = text
            .lines()
            .filter_map(|l| l.split(',').nth(2))
            .find_map(|s| s.trim().trim_end_matches(" MB").parse::<u64>().ok())
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(16 * 1024 * 1024 * 1024);

        let render_nodes = glob_dev("/dev/dri/renderD*");
        // Heuristic: APU (Ryzen AI / Radeon 8xxS iGPU) shares host RAM.
        let unified = render_nodes.is_empty();

        Some(Accelerator {
            backend: Backend::Amd,
            name: "AMD GPU/APU (rocm)".into(),
            mem_total,
            mem_bandwidth_gbps: if unified { 256.0 } else { 1000.0 },
            devices: render_nodes,
            unified_with_host: unified,
            custom_accel_id: None,
        })
    }
}
