//! Custom accelerator probes, loaded from ~/.config/accelrun/probes.toml
//! (TOML subset parsed by hand — no serde) so new hardware needs no recompile.

use super::*;

pub struct CustomProbes;

struct CustomProbe {
    id: String,
    device_glob: String,
    mem_gib: u64,
    unified: bool,
}

/// Tiny TOML-subset parser: handles [[probe]] tables with id/device_glob/mem_gib/unified.
fn parse_probes(text: &str) -> Vec<CustomProbe> {
    let mut out = vec![];
    let mut cur: Option<CustomProbe> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line == "[[probe]]" {
            if let Some(p) = cur.take() {
                out.push(p);
            }
            cur = Some(CustomProbe {
                id: String::new(),
                device_glob: String::new(),
                mem_gib: 0,
                unified: false,
            });
            continue;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if let Some(p) = cur.as_mut() {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "id" => p.id = v.to_string(),
                    "device_glob" => p.device_glob = v.to_string(),
                    "mem_gib" => p.mem_gib = v.parse().unwrap_or(0),
                    "unified" => p.unified = v == "true",
                    _ => {}
                }
            }
        }
    }
    if let Some(p) = cur.take() {
        out.push(p);
    }
    out
}

impl Probe for CustomProbes {
    fn detect(&self) -> Option<Accelerator> {
        let home = std::env::var_os("HOME")?;
        let path = std::path::Path::new(&home).join(".config/accelrun/probes.toml");
        let text = std::fs::read_to_string(path).ok()?;
        parse_probes(&text).into_iter().find_map(|p| {
            let nodes = glob_dev(&p.device_glob);
            if nodes.is_empty() {
                return None;
            }
            Some(Accelerator {
                backend: Backend::Custom,
                name: p.id.clone(),
                mem_total: p.mem_gib * 1024 * 1024 * 1024,
                mem_bandwidth_gbps: 500.0,
                devices: nodes,
                unified_with_host: p.unified,
                custom_accel_id: Some(p.id),
            })
        })
    }
}
