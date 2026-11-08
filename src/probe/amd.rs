//! AMD probe: /dev/kfd + amdgpu sysfs (primary), rocm-smi fallback.
//!
//! Attach model (differs from NVIDIA — see docs/runbook.md):
//! - `/dev/kfd` is THE compute interface (the HSA runtime's queue path),
//!   shared by all GPUs; `/dev/dri/renderD*` are the per-GPU DRM nodes
//!   (buffer objects / GEM). ROCm compute needs BOTH — AMD's own
//!   toolkit-free docker recipe is exactly `--device /dev/kfd
//!   --device /dev/dri`. The old probe presence-checked kfd but never
//!   injected it, which would have broken in-container compute ("no GPU
//!   agents" — the AMD analog of NVIDIA's "No devices were found").
//! - amdgpu is an in-tree kernel driver: nothing to probe for on the
//!   module side, and firmware loads on the HOST kernel, never in the
//!   container.
//! - sysfs (`mem_info_vram_total` / `mem_info_gtt_total`, in bytes) is the
//!   stable kernel interface; rocm-smi's CSV format drifts across ROCm
//!   releases, so it is only a fallback for hosts without the sysfs nodes.

use super::*;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct AmdProbe;

/// Read a whitespace-padded u64 from a sysfs node.
fn read_u64(p: &Path) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// Read a sysfs node and trim whitespace (None if missing/unreadable).
fn read_trimmed(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// `cardN` directories (N all digits — skips `cardN-DP-1` connector nodes)
/// whose driver symlink is amdgpu. Skips non-AMD cards (i915/nouveau...).
fn amdgpu_cards() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let digits = match name.strip_prefix("card") {
                Some(d) => d,
                None => continue,
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let dev = e.path().join("device");
            let driver = std::fs::read_link(dev.join("driver"))
                .ok()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()));
            if driver.as_deref() == Some("amdgpu") {
                out.push(dev);
            }
        }
    }
    out.sort();
    out
}

/// Integrated/unified heuristic — no single driver flag exists, so two
/// signals (checked in this order):
/// 1. PCI class `0x0380xx` ("Display controller, other") is used only by
///    integrated AMD GPUs (Strix Halo class); discrete cards report
///    `0x0300xx`, so this signal never misclassifies a discrete card.
/// 2. Otherwise a small VRAM "total" is the APU BIOS carve-out signature:
///    APUs expose a 512 MB–4 GB carve-out while every ROCm-capable
///    discrete card has >= 4 GiB dedicated.
/// Neither signal present -> discrete (the conservative default: no
/// unified-memory cgroup enforcement claim is made, same as NVIDIA VRAM).
fn is_unified(pci_class: Option<u32>, vram_total: Option<u64>) -> bool {
    if let Some(c) = pci_class {
        if (c >> 8) == 0x0380 {
            return true;
        }
    }
    matches!(vram_total, Some(v) if v < 4 * (1u64 << 30))
}

/// rocm-smi fallback for hosts without the sysfs memory nodes (ancient
/// kernels): first line whose third CSV field parses as MB. Deliberately
/// the same permissive parse the old probe used — sysfs is the primary
/// source, and this only sets the ranking/filter budget.
fn rocm_smi_vram_bytes() -> Option<u64> {
    let out = Command::new("rocm-smi")
        .args(["--showmeminfo", "vram", "--csv"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines()
        .filter_map(|l| l.split(',').nth(2))
        .find_map(|s| s.trim().trim_end_matches(" MB").parse::<u64>().ok())
        .map(|mb| mb * 1024 * 1024)
}

impl Probe for AmdProbe {
    fn detect(&self) -> Option<Accelerator> {
        if !Path::new("/dev/kfd").exists() {
            return None;
        }
        // Every renderD node is passed (whole-host passthrough, same
        // philosophy as NVIDIA's all numbered nodes); kfd is shared by all
        // GPUs and always included.
        let mut devices = vec!["/dev/kfd".to_string()];
        devices.extend(glob_dev("/dev/dri/renderD*"));

        // Best card by memory size (vram + gtt), sysfs-first.
        // (vram, gtt, pci class, product name)
        let mut best: Option<(u64, u64, Option<u32>, String)> = None;
        for dev in amdgpu_cards() {
            let vram = read_u64(&dev.join("mem_info_vram_total")).unwrap_or(0);
            let gtt = read_u64(&dev.join("mem_info_gtt_total")).unwrap_or(0);
            if vram == 0 && gtt == 0 {
                continue; // not a memory-bearing GPU (or unknown layout)
            }
            let pci_class = read_trimmed(&dev.join("class"))
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok());
            // product_name comes from the FRU (server cards only); empty otherwise
            let product = read_trimmed(&dev.join("product_name")).unwrap_or_default();
            if best.as_ref().map_or(true, |(bv, bg, _, _)| vram + gtt > bv + bg) {
                best = Some((vram, gtt, pci_class, product));
            }
        }

        let (vram, gtt, pci_class, product) = match best {
            Some(b) => b,
            None => (0, 0, None, String::new()),
        };
        let unified = is_unified(pci_class, if vram > 0 { Some(vram) } else { None });
        // Unified (APU): GTT — the system-RAM pool the GPU actually
        // allocates from — is the real capacity; discrete: dedicated VRAM.
        // Last resort: rocm-smi, then the old 16 GiB default.
        let mem_total = if unified {
            if gtt > 0 {
                gtt
            } else {
                rocm_smi_vram_bytes().unwrap_or(16 << 30)
            }
        } else if vram > 0 {
            vram
        } else {
            rocm_smi_vram_bytes().unwrap_or(16 << 30)
        };
        let name = if !product.is_empty() {
            product
        } else {
            "AMD GPU/APU (amdgpu)".to_string()
        };
        Some(Accelerator {
            backend: Backend::Amd,
            name,
            mem_total,
            // Bandwidth is a ranking heuristic only (the real fleet spans
            // ~250-5300 GB/s; exact values do not change correctness).
            mem_bandwidth_gbps: if unified { 256.0 } else { 1000.0 },
            devices,
            unified_with_host: unified,
            custom_accel_id: None,
        })
    }
}

/// Human-readable diagnostics for `accelrun probe`. Empty (prints nothing)
/// on hosts with no AMD hardware, so non-AMD users see no noise.
pub fn diagnostics() -> Vec<String> {
    if !Path::new("/dev/kfd").exists() && amdgpu_cards().is_empty() {
        return vec![];
    }
    let mut out = vec![format!(
        "/dev/kfd: {}",
        if Path::new("/dev/kfd").exists() {
            "present".to_string()
        } else {
            "ABSENT — no ROCm compute interface".to_string()
        }
    )];
    out.push(format!(
        "/dev/dri/renderD*: {:?}",
        glob_dev("/dev/dri/renderD*")
    ));
    for dev in amdgpu_cards() {
        let card = dev
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let class = read_trimmed(&dev.join("class")).unwrap_or_else(|| "?".into());
        let vram = read_u64(&dev.join("mem_info_vram_total"));
        let gtt = read_u64(&dev.join("mem_info_gtt_total"));
        let product = read_trimmed(&dev.join("product_name")).unwrap_or_default();
        out.push(format!(
            "{} class={} vram={} gtt={} {}",
            card,
            class,
            vram.map(fmt_gib).unwrap_or_else(|| "?".into()),
            gtt.map(fmt_gib).unwrap_or_else(|| "?".into()),
            product
        ));
    }
    for tool in ["rocm-smi", "amd-smi", "rocminfo"] {
        let status = match Command::new(tool).arg("--version").output() {
            Ok(_) => "present",
            Err(_) => "not found",
        };
        out.push(format!("{}: {}", tool, status));
    }
    out
}

/// e.g. 17179869184 -> "16.0GiB" (diagnostics only).
fn fmt_gib(b: u64) -> String {
    format!("{:.1}GiB", b as f64 / (1u64 << 30) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_by_display_controller_class() {
        // Integrated GPUs (Strix Halo class) report PCI class 0x0380xx;
        // discrete cards report 0x0300xx.
        assert!(is_unified(Some(0x038000), None));
        assert!(is_unified(Some(0x0380ff), Some(64 << 30)));
        assert!(!is_unified(Some(0x030000), Some(16 << 30)));
    }

    #[test]
    fn unified_by_vram_carveout_signature() {
        // APUs expose a small BIOS VRAM carve-out; every ROCm-capable
        // discrete card has >= 4 GiB dedicated.
        assert!(is_unified(None, Some(512 << 20)));
        assert!(is_unified(Some(0x030000), Some(2 << 30)));
        assert!(!is_unified(None, Some(8 << 30)));
    }

    #[test]
    fn unknown_defaults_to_discrete() {
        // Neither signal exists -> discrete: no unified-memory cgroup
        // enforcement claim is made (same conservative posture as NVIDIA).
        assert!(!is_unified(None, None));
    }
}
