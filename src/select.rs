//! Backend selection policy.

use crate::probe::{Accelerator, Probe};

pub struct Request {
    pub min_mem_bytes: u64,
    /// For large-model workloads: prefer unified RAM over raw bandwidth.
    pub prefer_unified: bool,
}

/// Returns (chosen, used_fallback). Never fails: CPU is always viable.
pub fn select(accels: Vec<Accelerator>, req: &Request) -> (Accelerator, bool) {
    let mut viable: Vec<Accelerator> = accels
        .into_iter()
        .filter(|a| a.mem_total >= req.min_mem_bytes)
        .collect();
    if viable.is_empty() {
        // No backend meets the requested budget. Degrade to the backend with
        // the MOST memory (a 15.9Gi GPU beats a CPU for a 16Gi request in
        // practice — the app gets ACCEL_MEM_BYTES as a budget hint and can
        // quantize/offload). Only if literally nothing exists, use CPU.
        return fallback_select(req);
    }
    viable.sort_by(|a, b| score(b, req).cmp(&score(a, req)));
    (viable.remove(0), false)
}

fn fallback_select(req: &Request) -> (Accelerator, bool) {
    let all: Vec<Accelerator> = crate::all_accels();
    // Prefer a real accelerator over CPU (CPU RAM always "wins" on raw bytes,
    // but for a GPU workload the GPU is the right fallback). Rank by
    // bandwidth first, memory second.
    let mut accels: Vec<Accelerator> = all
        .iter()
        .filter(|a| a.backend != crate::probe::Backend::Cpu)
        .cloned()
        .collect();
    if accels.is_empty() {
        accels = all;
    }
    let best = accels
        .into_iter()
        .max_by_key(|a| (score(a, req) as u128, a.mem_total));
    match best {
        Some(a) => (a, true),
        None => (
            crate::probe::cpu::CpuProbe
                .detect()
                .expect("cpu probe always succeeds"),
            true,
        ),
    }
}

fn score(a: &Accelerator, req: &Request) -> u64 {
    let mult = if req.prefer_unified && a.unified_with_host { 2.0 } else { 1.0 };
    (a.mem_bandwidth_gbps * mult * 1000.0) as u64
}
