//! Backend selection policy.

use crate::probe::{Accelerator, Backend, Probe};

pub struct Request {
    pub min_mem_bytes: u64,
    /// For large-model workloads: prefer unified RAM over raw bandwidth.
    pub prefer_unified: bool,
}

/// Returns (chosen, degraded). Never fails: CPU is always viable.
///
/// Policy (the model's rule, implemented exactly — README, decisions.md):
///   1. best-scoring *accelerator* that meets the budget   -> degraded=false
///   2. best-scoring accelerator anyway (budget missed)    -> degraded=true
///   3. CPU — only when no accelerator exists               -> false/true
/// CPU never wins merely because its RAM happens to meet a budget that an
/// accelerator misses: the pre-fix `viable = mem_total >= budget` filter
/// made exactly that happen *silently* (the NV4ads `--mem 8Gi` case), which
/// is this project's own ProbeFalseNegative failure mode ("GPU present but
/// not detected -> silently ran on CPU").
pub fn select(accels: Vec<Accelerator>, req: &Request) -> (Accelerator, bool) {
    let mut meeting: Vec<Accelerator> = vec![];
    let mut short: Vec<Accelerator> = vec![];
    let mut cpu_meeting: Option<Accelerator> = None;
    let mut cpu_short: Option<Accelerator> = None;
    for a in accels {
        let meets = a.mem_total >= req.min_mem_bytes;
        if a.backend == Backend::Cpu {
            if meets {
                cpu_meeting = Some(a);
            } else {
                cpu_short = Some(a);
            }
        } else if meets {
            meeting.push(a);
        } else {
            short.push(a);
        }
    }
    // Rank within a tier: score desc (bandwidth, --unified doubling), then
    // mem_total desc as tie-break (a 15.9-GiB GPU beats a 15-GiB one for a
    // 16-GiB request).
    let rank = |mut v: Vec<Accelerator>| -> Option<Accelerator> {
        v.sort_by(|a, b| {
            score(b, req)
                .cmp(&score(a, req))
                .then(b.mem_total.cmp(&a.mem_total))
        });
        v.into_iter().next()
    };
    if let Some(a) = rank(meeting) {
        return (a, false);
    }
    // Graceful degradation: a real accelerator beats CPU even when short on
    // VRAM — the app still gets ACCEL_MEM_BYTES as the budget hint and can
    // quantize/offload; the launcher warns loudly (degraded=true).
    if let Some(a) = rank(short) {
        return (a, true);
    }
    // CPU only when no accelerator exists (the README's promise).
    if let Some(a) = cpu_meeting {
        return (a, false);
    }
    if let Some(a) = cpu_short {
        return (a, true);
    }
    // Not reachable via main (CpuProbe always succeeds) — stay total anyway.
    let cpu = crate::probe::cpu::CpuProbe
        .detect()
        .expect("cpu probe always succeeds");
    (cpu, true)
}

fn score(a: &Accelerator, req: &Request) -> u64 {
    let mult = if req.prefer_unified && a.unified_with_host { 2.0 } else { 1.0 };
    (a.mem_bandwidth_gbps * mult * 1000.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(backend: Backend, mem: u64, bw: f64, unified: bool) -> Accelerator {
        Accelerator {
            backend,
            name: "T".into(),
            mem_total: mem,
            mem_bandwidth_gbps: bw,
            devices: vec![],
            unified_with_host: unified,
            custom_accel_id: None,
        }
    }

    fn req(mem: u64) -> Request {
        Request {
            min_mem_bytes: mem,
            prefer_unified: false,
        }
    }

    #[test]
    fn gpu_meeting_budget_wins_over_cpu() {
        let (a, fb) = select(
            vec![
                acc(Backend::Nvidia, 16 << 30, 1000.0, false),
                acc(Backend::Cpu, 33 << 30, 50.0, true),
            ],
            &req(12 << 30),
        );
        assert_eq!(a.backend, Backend::Nvidia);
        assert!(!fb);
    }

    #[test]
    fn gpu_short_on_budget_beats_cpu_that_meets_it() {
        // The planned NV4ads session case: 4-GiB GPU, 16-GiB host RAM,
        // --mem 8Gi. Pre-fix behavior: silent CPU (CPU RAM made `viable`
        // non-empty). Post-fix: the GPU, loudly degraded.
        let (a, fb) = select(
            vec![
                acc(Backend::Amd, 4 << 30, 1000.0, false),
                acc(Backend::Cpu, 16 << 30, 50.0, true),
            ],
            &req(8 << 30),
        );
        assert_eq!(a.backend, Backend::Amd);
        assert!(fb);
    }

    #[test]
    fn cpu_only_when_no_accelerator_exists() {
        let (a, fb) = select(
            vec![acc(Backend::Cpu, 16 << 30, 50.0, true)],
            &req(8 << 30),
        );
        assert_eq!(a.backend, Backend::Cpu);
        assert!(!fb);
    }

    #[test]
    fn cpu_short_budget_is_degraded() {
        let (a, fb) = select(
            vec![acc(Backend::Cpu, 4 << 30, 50.0, true)],
            &req(8 << 30),
        );
        assert_eq!(a.backend, Backend::Cpu);
        assert!(fb);
    }

    #[test]
    fn meeting_accelerator_outranks_short_one() {
        // Tier 1 (meeting) always outranks tier 2 (short), regardless of BW.
        let (a, fb) = select(
            vec![
                acc(Backend::Amd, 4 << 30, 1000.0, false),
                acc(Backend::Nvidia, 32 << 30, 1000.0, false),
            ],
            &req(8 << 30),
        );
        assert_eq!(a.backend, Backend::Nvidia);
        assert!(!fb);
    }
}
