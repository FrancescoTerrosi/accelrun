//! CPU fallback: always available. Unified with host by definition.

use super::*;

pub struct CpuProbe;

impl Probe for CpuProbe {
    fn detect(&self) -> Option<Accelerator> {
        let mem = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|kb| kb * 1024)
            })
            .unwrap_or(8 * 1024 * 1024 * 1024);
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        Some(Accelerator {
            backend: Backend::Cpu,
            name: format!("CPU x{}", cpus),
            mem_total: mem,
            mem_bandwidth_gbps: 50.0, // DDR5-class
            devices: vec![],
            unified_with_host: true,
            custom_accel_id: None,
        })
    }
}
