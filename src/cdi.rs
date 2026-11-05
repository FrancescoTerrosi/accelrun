//! CDI spec generation (Container Device Interface, vendor-neutral).
//! Emits accel.local/gpu=gpu0 with env + deviceNodes + cgroupRules.
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

    // Unified-memory backends: GPU memory is host RAM, so cap via cgroup v2.
    let cgroup_rules = if acc.unified_with_host {
        format!("[{{\"path\":\"memory.max\",\"value\":\"{}\"}}]", mem_budget)
    } else {
        "[]".to_string()
    };

    let spec = format!(
        r#"{{
  "cdiVersion": "0.8.0",
  "kind": "accel.local/gpu",
  "devices": [
    {{
      "name": "gpu0",
      "containerEdits": {{
        "env": [{}],
        "deviceNodes": [{}],
        "cgroupRules": {}
      }}
    }}
  ]
}}
"#,
        env.join(",\n        "),
        device_nodes.join(",\n        "),
        cgroup_rules
    );

    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join("accel.json");
    std::fs::write(&path, spec)?;
    Ok(path)
}
