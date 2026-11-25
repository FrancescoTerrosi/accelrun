//! CDI spec generation (Container Device Interface, vendor-neutral).
//! Emits accel.local/gpu=gpu0 with env + deviceNodes only.
//!
//! Memory limits are deliberately NOT in this spec: the CDI schema has no
//! field for cgroup memory rules (`deviceCgroupRules` is device-node rwm
//! only, not `memory.max`). Unified-memory enforcement is expressed with
//! `docker --memory` in oci.rs — applied in BOTH direct and `--cdi` modes —
//! which is the launcher's real lever and lands in the container's cgroup v2
//! `memory.max`. This spec is vendor-neutral device publication only.
//! `install_cdi_spec` copies it into a daemon-scanned directory for `--cdi`.
//! JSON is written by hand — the structure is fixed and small.

use crate::probe::{env_for, Accelerator};
use std::path::Path;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn write_cdi_spec(acc: &Accelerator, mem_budget: u64, out_dir: &Path) -> std::io::Result<std::path::PathBuf> {
    let env: Vec<String> = env_for(acc, mem_budget)
        .into_iter()
        .map(|(k, v)| format!("\"{}={}\"", k, esc(&v)))
        .collect();

    let device_nodes: Vec<String> = acc
        .devices
        .iter()
        .map(|d| format!("{{\"path\":\"{}\",\"permissions\":\"rwm\"}}", esc(d)))
        .collect();

    let spec = format!(
        r#"{{
  "cdiVersion": "0.8.0",
  "kind": "accel.local/gpu",
  "devices": [
    {{
      "name": "gpu0",
      "containerEdits": {{
        "env": [{}],
        "deviceNodes": [{}]
      }}
    }}
  ]
}}
"#,
        env.join(",\n        "),
        device_nodes.join(",\n        "),
    );

    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join("accel.json");
    std::fs::write(&path, spec)?;
    Ok(path)
}

/// Directories the Docker daemon scans for CDI specs, in priority order.
/// `/var/run/cdi` is the runtime-generated location (highest priority);
/// `/etc/cdi` is the static fallback. Both typically require root to write.
const CDI_DAEMON_DIRS: [&str; 2] = ["/var/run/cdi", "/etc/cdi"];

/// Install the spec into a directory the Docker daemon actually scans so that
/// `--device cdi:accel.local/gpu=gpu0` can resolve. Returns the installed
/// path, or a remediation-bearing error if neither directory is writable.
pub fn install_cdi_spec(acc: &Accelerator, mem_budget: u64) -> Result<std::path::PathBuf, String> {
    let mut last_err: Option<std::io::Error> = None;
    for dir in CDI_DAEMON_DIRS {
        match write_cdi_spec(acc, mem_budget, std::path::Path::new(dir)) {
            Ok(path) => return Ok(path),
            Err(e) => last_err = Some(e),
        }
    }
    Err(format!(
        "could not install the CDI spec into {} ({}) — run with sudo, or copy \
         /tmp/accelrun-cdi/accel.json into one of those directories and reload docker",
        CDI_DAEMON_DIRS.join(" or "),
        last_err.map(|e| e.to_string()).unwrap_or_else(|| "permission denied".into()),
    ))
}
