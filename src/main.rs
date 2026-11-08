//! accelrun: portable accelerator resource abstraction for containers.
//!
//! Usage:
//!   accelrun probe                                  # list detected backends
//!   accelrun run --mem 16Gi [--unified] --image IMG -- cmd...

mod cdi;
mod oci;
mod probe;
mod select;
mod sys;

use probe::Probe;

fn all_probes() -> Vec<Box<dyn Probe>> {
    vec![
        Box::new(probe::nvidia::NvidiaProbe),
        Box::new(probe::amd::AmdProbe),
        Box::new(probe::apple::AppleProbe),
        Box::new(probe::custom::CustomProbes),
        Box::new(probe::cpu::CpuProbe),
    ]
}

pub(crate) fn all_accels() -> Vec<probe::Accelerator> {
    all_probes().into_iter().filter_map(|p| p.detect()).collect()
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    };
    std::process::exit(code);
}

fn run(argv: &[String]) -> Result<i32, String> {
    // "run" subcommand is optional: `accelrun --mem 12Gi --image X -- cmd`
    // is equivalent to `accelrun run --mem 12Gi --image X -- cmd`.
    let (sub, rest) = match argv.first().map(|s| s.as_str()) {
        Some("probe") => ("probe", &argv[1..]),
        Some("faults") => ("faults", &argv[1..]),
        Some("run") => ("run", &argv[1..]),
        Some(other) if other.starts_with('-') => ("run", argv),
        Some(other) => return Err(format!("unknown subcommand '{}'; try: probe | run | faults", other)),
        None => return Err("usage: accelrun <probe|run|faults> ...".into()),
    };
    match sub {
        "probe" => {
            for p in all_probes() {
                if let Some(acc) = p.detect() {
                    println!(
                        "{:<8} {:<28} mem={:>14} BW={:>7.0} GB/s unified={} devices={:?}",
                        acc.backend.as_env(),
                        acc.name,
                        acc.mem_total,
                        acc.mem_bandwidth_gbps,
                        acc.unified_with_host,
                        acc.devices
                    );
                }
            }
            println!("--- diagnostics ---");
            for line in probe::nvidia::diagnostics() {
                if !line.is_empty() {
                    println!("{}", line);
                }
            }
            // amd::diagnostics() returns [] on hosts with no AMD hardware
            // (no /dev/kfd and no amdgpu sysfs cards), so this prints nothing
            // on NVIDIA-only machines.
            for line in probe::amd::diagnostics() {
                if !line.is_empty() {
                    println!("{}", line);
                }
            }
            Ok(0)
        }
        "run" => {
            let args = oci::parse_run(rest)?;
            let mem = oci::parse_mem(&args.mem)?;
            let accels = all_accels();
            let req = select::Request {
                min_mem_bytes: mem,
                prefer_unified: args.unified,
            };
            let (acc, fallback) = select::select(accels, &req);
            if fallback {
                // Loud and specific: name the degraded backend and the gap.
                // (The pre-fix message said "fell back to CPU" even when a
                // real GPU was the degraded target — and the worst case, CPU
                // silently winning a budget an accelerator missed, warned not
                // at all; see select.rs.)
                eprintln!(
                    "[accelrun] WARNING: no backend met the {}-byte budget; degraded to backend={} ({} bytes) — ACCEL_MEM_BYTES stays the requested hint",
                    mem, acc.backend.as_env(), acc.mem_total
                );
            }
            let cdi_dir = std::path::PathBuf::from("/tmp/accelrun-cdi");
            let path = cdi::write_cdi_spec(&acc, mem, &cdi_dir)
                .map_err(|e| format!("CDI spec write failed: {}", e))?;
            println!("[accelrun] CDI spec (for --cdi mode): {}", path.display());
            if acc.unified_with_host {
                // Enforce-vs-hint asymmetry (docs/vision.md): unified backends
                // get a real cap via the launcher's actual levers — docker
                // --memory in direct mode (oci.rs), the CDI spec's
                // cgroupRules in --cdi mode (cdi.rs). The old CLI-side
                // cgroup write capped the CLI's own process; removed.
                println!(
                    "[accelrun] unified backend: budget enforced via docker --memory ({} bytes)",
                    mem
                );
            }
            // fault check after selection
            if sys::detect(sys::Fault::ProbeFalseNegative, acc.backend.as_env()) {
                eprintln!(
                    "[accelrun] FAULT detected: {} -> {}",
                    sys::Fault::ProbeFalseNegative.description(),
                    sys::Fault::ProbeFalseNegative.mitigation()
                );
            }
            if args.cdi {
                println!("[accelrun] mode=cdi (requires Docker>=25 + spec in daemon CDI dir)");
            } else {
                println!("[accelrun] mode=direct (env + --device injection)");
            }
            oci::launch(&args, &acc, mem)?;
            Ok(0)
        }
        "faults" => {
            for f in [
                sys::Fault::ProbeFalseNegative,
                sys::Fault::UnifiedHostOOM,
                sys::Fault::VramOvercommit,
                sys::Fault::BackendCrashMidRun,
                sys::Fault::NumericDrift,
            ] {
                println!("- {:?}\n    {}\n    mitigation: {}", f, f.description(), f.mitigation());
            }
            Ok(0)
        }
        _ => unreachable!(),
    }
}
