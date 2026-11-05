//! Core abstraction contract: an accelerator is a backend + memory + devices.
//! The container never sees a physical device, only the env contract.

pub mod amd;
pub mod apple;
pub mod cpu;
pub mod custom;
pub mod nvidia;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Nvidia,
    Amd,
    Metal,
    Custom,
    Cpu,
}

impl Backend {
    pub fn as_env(&self) -> &'static str {
        match self {
            Backend::Nvidia => "nvidia",
            Backend::Amd => "amd",
            Backend::Metal => "metal",
            Backend::Custom => "custom",
            Backend::Cpu => "cpu",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Accelerator {
    pub backend: Backend,
    pub name: String,
    /// Total memory the backend can address, in bytes. For unified-memory
    /// backends this is a *budget slice* of host RAM, enforced via cgroups.
    pub mem_total: u64,
    pub mem_bandwidth_gbps: f64,
    /// Device nodes to pass into the container, if any.
    pub devices: Vec<String>,
    /// true on Apple UMA / AMD APU / CXL: GPU memory IS host RAM.
    pub unified_with_host: bool,
    pub custom_accel_id: Option<String>,
}

pub trait Probe {
    fn detect(&self) -> Option<Accelerator>;
}

/// Environment contract injected into the container. The workload reads
/// these and dispatches to Triton / MLIR / SYCL / WebGPU / ONNX-RT accordingly.
pub fn env_for(acc: &Accelerator, mem_budget: u64) -> Vec<(String, String)> {
    vec![
        ("ACCEL_BACKEND".into(), acc.backend.as_env().into()),
        ("ACCEL_MEM_BYTES".into(), mem_budget.to_string()),
        ("ACCEL_UNIFIED".into(), acc.unified_with_host.to_string()),
        (
            "ACCEL_DEVICE".into(),
            acc.devices.first().cloned().unwrap_or_default(),
        ),
        ("ACCEL_NAME".into(), acc.name.clone()),
    ]
}

/// Minimal glob: supports trailing "*" patterns like "/dev/nvidia[0-9]*" and "/dev/dri/renderD*".
/// Good enough for device discovery; not a general glob engine.
pub fn glob_dev(pattern: &str) -> Vec<String> {
    let (dir, file_pat) = match pattern.rfind('/') {
        Some(i) => (&pattern[..i + 1], &pattern[i + 1..]),
        None => ("", pattern),
    };
    let read = std::fs::read_dir(if dir.is_empty() { "." } else { dir }).ok();
    let mut out = vec![];
    if let Some(rd) = read {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if glob_match(file_pat, &name) {
                out.push(format!("{}{}", dir, name));
            }
        }
    }
    out.sort();
    out
}

fn glob_match(pat: &str, name: &str) -> bool {
    // support: '*' (any run), classes '[...]' with ranges ('[0-9]', '[a-z]'), literal chars
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fn rec(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| rec(&p[1..], &n[i..])),
            Some('[') => {
                // class: literal chars and ranges 'lo-hi' until ']' (bug fix:
                // the old parser required exactly a 2-char class, so [0-9]
                // and [a-z] NEVER matched — glob_dev("/dev/nvidia[0-9]*")
                // always returned [] and the raw-node GPU injection silently
                // passed only the control nodes).
                if n.is_empty() {
                    return false;
                }
                let close = match p.iter().position(|&c| c == ']') {
                    Some(i) if i >= 2 => i,
                    _ => return false,
                };
                let c = n[0];
                let mut i = 1;
                let mut ok = false;
                while i < close {
                    if i + 2 < close && p[i + 1] == '-' {
                        if p[i] <= c && c <= p[i + 2] {
                            ok = true;
                        }
                        i += 3;
                    } else {
                        if p[i] == c {
                            ok = true;
                        }
                        i += 1;
                    }
                }
                ok && rec(&p[close + 1..], &n[1..])
            }
            Some(c) => !n.is_empty() && n[0] == *c && rec(&p[1..], &n[1..]),
        }
    }
    rec(&p, &n)
}
