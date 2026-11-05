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
                eprintln!("[accelrun] WARNING: fell back to CPU (possible fault: ProbeFalseNegative)");
            }
            let cdi_dir = std::path::PathBuf::from("/tmp/accelrun-cdi");
            let path = cdi::write_cdi_spec(&acc, mem, &cdi_dir)
                .map_err(|e| format!("CDI spec write failed: {}", e))?;
            println!("[accelrun] CDI spec (for --cdi mode): {}", path.display());
            if acc.unified_with_host {
                if let Err(e) = sys::apply_memory_cap("default", mem) {
                    eprintln!("[accelrun] cgroup cap skipped (expected w/o root): {}", e);
                } else {
                    println!("[accelrun] capped unified RAM at {} bytes", mem);
                }
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
