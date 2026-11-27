//! OCI/docker wrapper + CLI arg parsing (hand-rolled, no clap).

use crate::probe::Accelerator;
use std::process::Command;

pub struct RunArgs {
    pub mem: String,
    pub unified: bool,
    pub cdi: bool,
    pub dry_run: bool,
    pub volumes: Vec<String>,
    pub name: Option<String>,
    pub network: Option<String>,
    pub detach: bool,
    pub ports: Vec<String>,
    pub envs: Vec<String>,
    pub restart: Option<String>,
    pub image: String,
    pub cmd: Vec<String>,
}

pub fn parse_mem(s: &str) -> Result<u64, String> {
    let (num, mult) = if let Some(n) = s.strip_suffix("Gi") {
        (n, 1u64 << 30)
    } else if let Some(n) = s.strip_suffix("Mi") {
        (n, 1u64 << 20)
    } else {
        (s, 1)
    };
    let n = num
        .parse::<u64>()
        .map_err(|_| format!("bad --mem value: {}", s))?;
    n.checked_mul(mult)
        .ok_or_else(|| format!("bad --mem value: {}", s))
}

/// Minimal parser for: run --mem 16Gi [--unified] [--name N] [--network NET]
///                     [-d] [-p H:CP]... [-e K=V]... [--restart POLICY]
///                     --image IMG -- cmd...
pub fn parse_run(argv: &[String]) -> Result<RunArgs, String> {
    let mut mem = "8Gi".to_string();
    let mut unified = false;
    let mut cdi = false;
    let mut dry_run = false;
    let mut volumes: Vec<String> = vec![];
    let mut name = None;
    let mut network = None;
    let mut detach = false;
    let mut ports: Vec<String> = vec![];
    let mut envs: Vec<String> = vec![];
    let mut restart = None;
    let mut image = None;
    let mut cmd = vec![];
    let mut i = 0;
    let mut seen_dashdash = false;
    while i < argv.len() {
        let a = &argv[i];
        if seen_dashdash {
            cmd.push(a.clone());
            i += 1;
            continue;
        }
        match a.as_str() {
            "--" => seen_dashdash = true,
            "--mem" => {
                i += 1;
                mem = argv.get(i).ok_or("--mem needs a value")?.clone();
            }
            "--unified" => unified = true,
            "--cdi" => cdi = true,
            "--dry-run" => dry_run = true,
            "--image" => {
                i += 1;
                image = Some(argv.get(i).ok_or("--image needs a value")?.clone());
            }
            "--volume" | "-v" => {
                i += 1;
                volumes.push(argv.get(i).ok_or("--volume needs a value")?.clone());
            }
            x if x.starts_with("--mem=") => mem = x[6..].to_string(),
            x if x.starts_with("--image=") => image = Some(x[8..].to_string()),
            x if x.starts_with("--volume=") => volumes.push(x[9..].to_string()),
            "--name" => {
                i += 1;
                name = Some(argv.get(i).ok_or("--name needs a value")?.clone());
            }
            "--network" | "--net" => {
                i += 1;
                network = Some(argv.get(i).ok_or("--network needs a value")?.clone());
            }
            "-d" | "--detach" => detach = true,
            "-p" | "--publish" => {
                i += 1;
                ports.push(argv.get(i).ok_or("-p needs a value")?.clone());
            }
            "-e" | "--env" => {
                i += 1;
                envs.push(argv.get(i).ok_or("-e needs KEY=VAL")?.clone());
            }
            "--restart" => {
                i += 1;
                restart = Some(argv.get(i).ok_or("--restart needs a policy")?.clone());
            }
            x if x.starts_with("--name=") => name = Some(x[7..].to_string()),
            x if x.starts_with("--network=") => network = Some(x[10..].to_string()),
            x if x.starts_with("--publish=") => ports.push(x[10..].to_string()),
            x if x.starts_with("--env=") => envs.push(x[6..].to_string()),
            x if x.starts_with("--restart=") => restart = Some(x[10..].to_string()),
            other if other.starts_with('-') => return Err(format!("unknown flag: {}", other)),
            other => return Err(format!(
                "unexpected positional '{}' before '--' (did you forget --image? full syntax: accelrun run --mem 12Gi [--volume SRC:DST[:ro]] --image IMG -- CMD...)",
                other
            )),
        }
        i += 1;
    }
    Ok(RunArgs {
        mem,
        unified,
        cdi,
        dry_run,
        volumes,
        name,
        network,
        detach,
        ports,
        envs,
        restart,
        image: image.ok_or("missing --image")?,
        cmd,
    })
}

/// True if the NVIDIA Container Toolkit is installed (required for --gpus).
fn nvidia_toolkit_present() -> bool {
    Command::new("nvidia-ctk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::path::Path::new("/usr/bin/nvidia-container-runtime").exists()
}

/// Push `host` -> `container` into `mounts`, deduplicating by container path
/// (docker refuses two mounts at the same container path — reachable when the
/// same lib exists in more than one host libdir).
fn push_mount(mounts: &mut Vec<(String, String)>, seen: &mut Vec<String>, host: &str, container: &str) {
    if seen.iter().any(|d| d == container) {
        return;
    }
    seen.push(container.to_string());
    mounts.push((host.to_string(), container.to_string()));
}

/// Host -> container bind mounts for the selected backend's driver userspace,
/// in the exact order direct mode emits them as `-v` args. Deduplicated by
/// container path. `launch` turns these into `-v host:container:ro`; `--cdi`
/// mode prints them as a hint of what a consumer must mount for compute.
///
/// - NVIDIA bare-metal: driver API + NVML + PTX-JIT/compiler libs, mirrored
///   into the Debian multiarch dir (Fedora/merge-usr hosts), plus nvidia-smi.
/// - NVIDIA WSL2 (`/dev/dxg`): the Windows driver's `/usr/lib/wsl/lib` + smi.
/// - AMD: ROCm userspace (hsa-runtime64/hsakmt/amdhip64/comgr, drm, elf, numa)
///   + host rocminfo/amd-smi/rocm-smi.
/// - CPU, Metal, custom: empty (image-borne or nothing to inject).
pub fn attach_mounts(acc: &Accelerator) -> Vec<(String, String)> {
    let mut mounts: Vec<(String, String)> = vec![];
    let mut seen: Vec<String> = vec![];

    match acc.backend {
        crate::probe::Backend::Nvidia => {
            if acc.devices.iter().any(|d| d == "/dev/dxg") {
                push_mount(&mut mounts, &mut seen, "/usr/lib/wsl/lib", "/usr/lib/wsl/lib");
                if std::path::Path::new("/usr/lib/wsl/lib/nvidia-smi").exists() {
                    push_mount(&mut mounts, &mut seen, "/usr/lib/wsl/lib/nvidia-smi", "/usr/bin/nvidia-smi");
                }
            } else {
                // Merge-usr hosts (Ubuntu/Debian: /lib -> /usr/lib) would mount
                // the same lib twice via different paths — skip libdirs that
                // canonicalize to an already-scanned one.
                let mut dirs_seen: Vec<String> = vec![];
                for libdir in ["/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/lib/x86_64-linux-gnu"] {
                    let canon = std::fs::canonicalize(libdir)
                        .map(|c| c.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| libdir.to_string());
                    if dirs_seen.contains(&canon) {
                        continue;
                    }
                    dirs_seen.push(canon);
                    for pat in [
                        "libnvidia-ml.so*",
                        "libcuda.so*",
                        "libnvidia-gpucomp.so*",
                        "libnvidia-ptxjitcompiler.so*",
                        "libnvidia-nvvm.so*",
                        "libnvjitlink.so*",
                        "libnvidia-compiler.so*",
                    ] {
                        for f in crate::probe::glob_dev(&format!("{}/{}", libdir, pat)) {
                            push_mount(&mut mounts, &mut seen, &f, &f);
                            // Cross-distro loader path: Debian/Ubuntu images do
                            // not search /usr/lib64, so mirror libs into the
                            // multiarch dir.
                            if libdir == "/usr/lib64" {
                                if let Some(name) = f.rsplit('/').next() {
                                    if !name.is_empty() {
                                        push_mount(&mut mounts, &mut seen, &f, &format!("/usr/lib/x86_64-linux-gnu/{}", name));
                                    }
                                }
                            }
                        }
                    }
                }
                if std::path::Path::new("/usr/bin/nvidia-smi").exists() {
                    push_mount(&mut mounts, &mut seen, "/usr/bin/nvidia-smi", "/usr/bin/nvidia-smi");
                }
            }
        }
        crate::probe::Backend::Amd => {
            let mut dirs_seen: Vec<String> = vec![];
            for libdir in [
                "/opt/rocm/lib",
                "/usr/lib/x86_64-linux-gnu",
                "/usr/lib64",
                "/lib/x86_64-linux-gnu",
            ] {
                let canon = std::fs::canonicalize(libdir)
                    .map(|c| c.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| libdir.to_string());
                if dirs_seen.contains(&canon) {
                    continue;
                }
                dirs_seen.push(canon);
                for pat in [
                    "libhsa-runtime64.so*",
                    "libhsakmt.so*",
                    "libamdhip64.so*",
                    "libamd_comgr.so*",
                    "libdrm_amdgpu.so*",
                    "libdrm.so*",
                    "libelf.so*",
                    "libnuma.so*",
                ] {
                    for f in crate::probe::glob_dev(&format!("{}/{}", libdir, pat)) {
                        push_mount(&mut mounts, &mut seen, &f, &f);
                        // Mirror into the multiarch dir (same cross-distro fix
                        // as NVIDIA): identity binds in /opt/rocm or /usr/lib64
                        // are invisible to a Debian-family loader.
                        if libdir != "/usr/lib/x86_64-linux-gnu" {
                            if let Some(name) = f.rsplit('/').next() {
                                if !name.is_empty() {
                                    push_mount(&mut mounts, &mut seen, &f, &format!("/usr/lib/x86_64-linux-gnu/{}", name));
                                }
                            }
                        }
                    }
                }
            }
            for tool in [
                "/opt/rocm/bin/rocminfo",
                "/usr/bin/rocminfo",
                "/opt/rocm/bin/amd-smi",
                "/usr/bin/amd-smi",
                "/opt/rocm/bin/rocm-smi",
                "/usr/bin/rocm-smi",
            ] {
                if std::path::Path::new(tool).exists() {
                    if let Some(name) = tool.rsplit('/').next() {
                        if !name.is_empty() {
                            push_mount(&mut mounts, &mut seen, tool, &format!("/usr/bin/{}", name));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    mounts
}

pub fn launch(args: &RunArgs, acc: &Accelerator, mem_bytes: u64) -> Result<(), String> {
    let mut cmd = Command::new("docker");
    cmd.arg("run");
    // --rm conflicts with --restart (docker refuses the combination) and is
    // pointless with -d: only add it for foreground, restart-less runs.
    if !args.detach && args.restart.is_none() {
        cmd.arg("--rm");
    }
    // passthrough -v/--volume mounts (e.g. to get the workload itself in)
    for v in &args.volumes {
        cmd.arg("-v").arg(v);
    }
    // naming / networking so accelrun-launched containers can be peers of a
    // compose network (e.g. a named LLM service container as a peer).
    if let Some(n) = &args.name {
        cmd.arg("--name").arg(n);
    }
    if let Some(net) = &args.network {
        cmd.arg("--network").arg(net);
    }
    if args.detach {
        cmd.arg("--detach");
    }
    for p in &args.ports {
        cmd.arg("-p").arg(p);
    }
    // app-level env on top of the ACCEL_* contract (e.g. OLLAMA_HOST)
    for e in &args.envs {
        cmd.arg("-e").arg(e);
    }
    if let Some(r) = &args.restart {
        cmd.arg("--restart").arg(r);
    }
    if args.cdi {
        // CDI path: needs Docker >= 25 and the spec installed in the daemon's
        // CDI dir (/etc/cdi or /var/run/cdi). Opt-in only.
        cmd.arg("--device").arg("cdi:accel.local/gpu=gpu0");
    } else {
        // Default path: direct injection — works on any Docker version.
        // Env contract:
        for (k, v) in crate::probe::env_for(acc, mem_bytes) {
            cmd.arg("-e").arg(format!("{}={}", k, v));
        }
        // Toolkit-FREE GPU attach: raw device nodes + driver libraries bind-mounted.
        // (--gpus all is only a last resort when NO nodes are visible —
        // restricted namespaces — and it does require the NVIDIA Container
        // Toolkit. WSL2 hosts have their own toolkit-free path: the dxg
<<<<<<< HEAD
        // branch in the Nvidia arm below.)
=======
        // branch below.)
>>>>>>> origin/main
        if !acc.devices.is_empty() {
            for d in &acc.devices {
                cmd.arg("--device").arg(d);
            }
<<<<<<< HEAD
            // docker's --device relaxes only the cgroup side; file perms still
            // win (AMD nodes are 0660 root:video/render; NVIDIA's 0666). Pass
            // each node's owning gid via --group-add (stat'd, not hardcoded).
            let mut gids: Vec<u32> = vec![];
            for d in &acc.devices {
                if let Ok(md) = std::fs::metadata(d) {
                    use std::os::unix::fs::MetadataExt;
                    let gid = md.gid();
                    if gid != 0 && !gids.contains(&gid) {
                        gids.push(gid);
                    }
                }
            }
            for gid in gids {
                cmd.arg("--group-add").arg(gid.to_string());
            }
            let mounts = attach_mounts(acc);
            match acc.backend {
                crate::probe::Backend::Nvidia => {
                    if acc.devices.iter().any(|d| d == "/dev/dxg") {
                        // WSL2 attach — mirrors nvidia-container-toolkit's WSL
                        // mode: dxg is the whole device story; the Windows
                        // driver ships its userspace under /usr/lib/wsl/lib,
                        // mounted :ro with LD_LIBRARY_PATH pointing at it.
                        // NB: docker -e overrides an image's LD_LIBRARY_PATH.
                        for (host, container) in &mounts {
                            cmd.arg("-v").arg(format!("{}:{}:ro", host, container));
                        }
                        cmd.arg("-e").arg("LD_LIBRARY_PATH=/usr/lib/wsl/lib");
                    } else {
                        // nvidiactl / uvm usually live alongside the numbered nodes
                        for extra in ["/dev/nvidiactl", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools"] {
                            if std::path::Path::new(extra).exists()
                                && !acc.devices.contains(&extra.to_string())
                            {
                                cmd.arg("--device").arg(extra);
                            }
                        }
                        for (host, container) in &mounts {
                            cmd.arg("-v").arg(format!("{}:{}:ro", host, container));
                        }
                    }
                    cmd.arg("-e").arg("NVIDIA_VISIBLE_DEVICES=all")
                       .arg("-e").arg("NVIDIA_DRIVER_CAPABILITIES=compute,utility");
                }
                crate::probe::Backend::Amd => {
                    // ROCm attach — the host KFD userspace is coupled to the
                    // host kernel's KFD ioctls (like libcuda to the NVIDIA
                    // driver), so it must be bind-mounted from the host. An
                    // image with its own ROCm stack gets these mounts
                    // shadowing it — keep host ROCm >= image ROCm
                    // (docs/runbook.md, AMD failure map).
                    for (host, container) in &mounts {
                        cmd.arg("-v").arg(format!("{}:{}:ro", host, container));
                    }
                }
                // Custom accelerators: raw nodes only — their userspace is
                // image-borne by design (docs/gemmini-case-study-plan.md and
                // the runbook); nothing NVIDIA/AMD-specific is injected.
                _ => {}
=======
            if acc.devices.iter().any(|d| d == "/dev/dxg") {
                // WSL2 attach, toolkit-free — mirrors nvidia-container-
                // toolkit's own WSL mode (libnvidia-container): the
                // paravirt dxg node is the whole device story (injected
                // by the generic --device loop above); the Windows
                // driver ships its userspace (libcuda, NVML, PTX JIT,
                // smi) as one directory, bind-mounted read-only at its
                // WSL identity path, with LD_LIBRARY_PATH pointing the
                // loader at it. Nothing else exists to inject: WSL has
                // no nvidiactl/uvm nodes, no host /usr/lib64 driver
                // globs, no /usr/bin smi of its own.
                // NB: docker -e cannot append — an image that pre-sets
                // LD_LIBRARY_PATH gets it overridden (runbook, WSL map).
                cmd.arg("-v").arg("/usr/lib/wsl/lib:/usr/lib/wsl/lib:ro");
                // smi parity with the bare-metal path: queryable at the
                // usual /usr/bin path (glibc images only, as ever).
                if std::path::Path::new("/usr/lib/wsl/lib/nvidia-smi").exists() {
                    cmd.arg("-v").arg("/usr/lib/wsl/lib/nvidia-smi:/usr/bin/nvidia-smi:ro");
                }
                cmd.arg("-e").arg("LD_LIBRARY_PATH=/usr/lib/wsl/lib");
            } else {
                // nvidiactl / uvm usually live alongside the numbered nodes
                for extra in ["/dev/nvidiactl", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools"] {
                    if std::path::Path::new(extra).exists() && !acc.devices.contains(&extra.to_string()) {
                        cmd.arg("--device").arg(extra);
                    }
                }
                // Mount driver libs so the container's CUDA can talk to the host driver.
                // Mirrors nvidia-container-cli's compute+utility set: ml (NVML), cuda
                // (driver API), ptxjitcompiler/gpucomp/nvvm/nvjitlink/compiler (JIT +
                // forward-compat pieces).
                for libdir in ["/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/lib/x86_64-linux-gnu"] {
                    for pat in [
                        "libnvidia-ml.so*",
                        "libcuda.so*",
                        "libnvidia-gpucomp.so*",
                        "libnvidia-ptxjitcompiler.so*",
                        "libnvidia-nvvm.so*",
                        "libnvjitlink.so*",
                        "libnvidia-compiler.so*",
                    ] {
                        for f in crate::probe::glob_dev(&format!("{}/{}", libdir, pat)) {
                            cmd.arg("-v").arg(format!("{}:{}:ro", f, f));
                            // Cross-distro loader paths: Debian/Ubuntu images do
                            // NOT search /usr/lib64 (their dynamic linker uses
                            // the multiarch dirs), so on a Fedora host the
                            // identity mount is invisible to dlopen/ld.so inside
                            // the container ("libcuda.so.1: cannot open shared
                            // object file"). Mirror the lib into the multiarch dir
                            // as a second read-only bind; Fedora-family images
                            // keep finding it via the identity mount in
                            // /usr/lib64.
                            if libdir == "/usr/lib64" {
                                if let Some(name) = f.rsplit('/').next() {
                                    if !name.is_empty() {
                                        cmd.arg("-v").arg(format!(
                                            "{}:/usr/lib/x86_64-linux-gnu/{}:ro",
                                            f, name
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                // nvidia-smi is driver userspace too (host-provided, glibc-linked):
                // mount it so non-CUDA images can query the GPU. NB: it will not
                // exec on musl images (alpine) — NVIDIA userspace needs glibc ld.so.
                if std::path::Path::new("/usr/bin/nvidia-smi").exists() {
                    cmd.arg("-v").arg("/usr/bin/nvidia-smi:/usr/bin/nvidia-smi:ro");
                }
>>>>>>> origin/main
            }
        } else if acc.backend == crate::probe::Backend::Nvidia {
            // Driver present but neither numbered nodes nor /dev/dxg visible
<<<<<<< HEAD
            // (a WSL2 host with a working driver takes the dxg direct path in
            // the nvidia arm instead; landing here means a restricted
            // namespace or a broken WSL driver setup): the only docker path
            // left is --gpus all, which needs the toolkit.
=======
            // (a WSL2 host with a working driver takes the dxg direct path
            // instead; landing here means a restricted namespace or a broken
            // WSL driver setup): the only docker path left is --gpus all,
            // which needs the toolkit.
>>>>>>> origin/main
            if nvidia_toolkit_present() {
                cmd.arg("--gpus").arg("all");
            } else {
                return Err(
                    "nvidia-smi lists a GPU but no /dev/nvidia* nodes and no /dev/dxg are visible, and \
                     the NVIDIA Container Toolkit is not installed. On WSL install the Windows NVIDIA \
                     driver (the distro then gets /dev/dxg + /usr/lib/wsl/lib); on bare metal check the \
                     driver loaded: `ls /dev/nvidia*`. The --gpus all fallback needs the toolkit \
                     (`nvidia-ctk --version`)."
                        .into(),
                );
            }
        }
    }
    // Memory policy — the model's enforce-vs-hint asymmetry
    // (docs/vision.md, decisions.md):
    // - unified backends are ENFORCED: docker --memory hard-caps the
    //   container's host RAM at the requested budget (lands in the
    //   container's cgroup v2 memory.max). docker's -m wants raw bytes (or
    //   lowercase "16g", NOT "16Gi").
    // - discrete VRAM is HINT-only (consumer cards have no MIG): no
    //   --memory, so the container's host RAM is no longer silently capped
    //   at the VRAM budget (the old unconditional flag made `--mem 2Gi` on
    //   a GPU host a 2-GiB-RAM container); ACCEL_MEM_BYTES carries the hint.
    // This --memory is applied in BOTH modes on purpose: it is the only
    // enforcement lever for unified backends, and CDI has no memory-limit
    // field (see cdi.rs) — so --cdi mode relies on this same docker --memory,
    // not on anything in the emitted spec. The cap lands in the container's
    // cgroup v2 memory.max.
    if acc.unified_with_host {
        cmd.arg("--memory").arg(mem_bytes.to_string());
    }
    cmd.arg(&args.image);
    cmd.args(&args.cmd);
    if args.dry_run {
        let mut parts: Vec<String> = vec![cmd.get_program().to_string_lossy().into_owned()];
        parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
        println!("[accelrun] DRY-RUN: {}", parts.join(" "));
        return Ok(());
    }    println!(
        "[accelrun] backend={} unified={} launching: docker run ... {}",
        acc.backend.as_env(),
        acc.unified_with_host,
        args.image
    );
    let status = cmd.status().map_err(|e| format!("failed to spawn docker: {}", e))?;
    if !status.success() {
        return Err(format!("container exited with {}", status));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

<<<<<<< HEAD
    /// Synthetic AMD accelerator -> dry-run argv assembly. Not hardware
    /// verification (TODO medium term): it proves the attach argv is
    /// built without panicking and lets a human eyeball the exact argv
    /// (`cargo test amd_dry_run -- --nocapture`). Run with --nocapture to
    /// inspect: expects --device /dev/kfd + renderD, ROCm lib mounts from
    /// whatever exists on THIS host (often just libdrm), and NO
    /// NVIDIA_VISIBLE_DEVICES / nvidia lib mounts on the AMD path.
    #[test]
    fn amd_dry_run_argv_assembles() {
        let acc = crate::probe::Accelerator {
            backend: crate::probe::Backend::Amd,
            name: "TEST gfx1100".into(),
            mem_total: 16 << 30,
            mem_bandwidth_gbps: 1000.0,
            devices: vec!["/dev/kfd".into(), "/dev/dri/renderD128".into()],
            unified_with_host: false,
        };
        let args = RunArgs {
            mem: "16Gi".into(),
            unified: false,
            cdi: false,
            dry_run: true,
            volumes: vec![],
            name: None,
            network: None,
            detach: false,
            ports: vec![],
            envs: vec![],
            restart: None,
            image: "python:3.12-slim".into(),
            cmd: vec!["python3".into(), "-c".into(), "1".into()],
        };
        launch(&args, &acc, 16 << 30).expect("dry-run launch must assemble argv");
    }

    /// Synthetic WSL2 NVIDIA accelerator -> dry-run argv assembly. Same
    /// honesty tier as amd_dry_run_argv_assembles: proves the WSL attach
    /// argv builds without panicking; it has NEVER run on WSL hardware (the
    /// author's WSL2 host owns the first gpu-smoke.py PASS — runbook, WSL
    /// section). Eyeball with `cargo test nvidia_wsl -- --nocapture`: expects
    /// --device /dev/dxg, -v /usr/lib/wsl/lib:/usr/lib/wsl/lib:ro, -e
=======
    /// Synthetic WSL2 NVIDIA accelerator -> dry-run argv assembly: proves
    /// the WSL attach argv builds without panicking. It has NEVER run on
    /// WSL hardware (the author's WSL2 host owns the first gpu-smoke.py PASS
    /// — runbook, WSL2 section). Eyeball with `cargo test nvidia_wsl --
    /// --nocapture`: expects --device /dev/dxg, -v
    /// /usr/lib/wsl/lib:/usr/lib/wsl/lib:ro, -e
>>>>>>> origin/main
    /// LD_LIBRARY_PATH=/usr/lib/wsl/lib, NVIDIA_VISIBLE_DEVICES=all — and
    /// NONE of the bare-metal artifacts (nvidiactl/uvm --device, /usr/lib64
    /// lib globs, /usr/bin/nvidia-smi identity mount).
    #[test]
    fn nvidia_wsl_dry_run_argv_assembles() {
        let acc = crate::probe::Accelerator {
            backend: crate::probe::Backend::Nvidia,
            name: "TEST WSL GPU".into(),
            mem_total: 12 << 30,
            mem_bandwidth_gbps: 1000.0,
            devices: vec!["/dev/dxg".into()],
            unified_with_host: false,
<<<<<<< HEAD
=======
            custom_accel_id: None,
>>>>>>> origin/main
        };
        let args = RunArgs {
            mem: "12Gi".into(),
            unified: false,
            cdi: false,
            dry_run: true,
            volumes: vec![],
            name: None,
            network: None,
            detach: false,
            ports: vec![],
            envs: vec![],
            restart: None,
            image: "python:3.12-slim".into(),
            cmd: vec!["python3".into(), "-c".into(), "1".into()],
        };
        launch(&args, &acc, 12 << 30).expect("dry-run launch must assemble argv");
    }
}
