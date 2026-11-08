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
    num.parse::<u64>()
        .map(|n| n * mult)
        .map_err(|_| format!("bad --mem value: {}", s))
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

/// Bind-mount `src` -> `dest` read-only, skipping duplicate destinations
/// (docker refuses two mounts at the same container path — reachable when
/// the same lib exists in more than one host libdir).
fn bind_ro(cmd: &mut Command, src: &str, dest: &str, seen: &mut Vec<String>) {
    if seen.iter().any(|d| d == dest) {
        return;
    }
    seen.push(dest.to_string());
    cmd.arg("-v").arg(format!("{}:{}:ro", src, dest));
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
        // (--gpus all is only used as a last resort when no nodes are visible,
        // e.g. WSL, and it does require the NVIDIA Container Toolkit.)
        if !acc.devices.is_empty() {
            for d in &acc.devices {
                cmd.arg("--device").arg(d);
            }
            // Device-node group permissions: docker's --device only relaxes
            // the *cgroup* side of access — the node's file permissions still
            // win (AMD nodes are 0660 root:video / root:render; NVIDIA's are
            // 0666). Pass each node's owning gid as a supplemental group so
            // non-root container users can open them (--group-add is AMD's
            // own documented fix; gids are stat'd from the nodes themselves
            // because the render gid is not standardized across distros).
            // Inert when nodes are 0666 root:root, so the verified NVIDIA argv
            // is unchanged on 0666 hosts.
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
            match acc.backend {
                crate::probe::Backend::Nvidia => {
                    // nvidiactl / uvm usually live alongside the numbered nodes
                    for extra in ["/dev/nvidiactl", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools"] {
                        if std::path::Path::new(extra).exists()
                            && !acc.devices.contains(&extra.to_string())
                        {
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
                    cmd.arg("-e").arg("NVIDIA_VISIBLE_DEVICES=all")
                       .arg("-e").arg("NVIDIA_DRIVER_CAPABILITIES=compute,utility");
                }
                crate::probe::Backend::Amd => {
                    // ROCm attach, toolkit-free — the direct mirror of the
                    // NVIDIA pattern: nodes are injected above; now the host
                    // ROCm userspace is bind-mounted read-only. The KFD
                    // userspace must come from the HOST (it is coupled to the
                    // host kernel's KFD ioctls), exactly like libcuda is
                    // coupled to the NVIDIA driver. The set is the AMD analog
                    // of nvidia-container-cli's compute+utility set:
                    //   libhsa-runtime64/libhsakmt — the KFD userspace
                    //                               (the "libcuda" analog)
                    //   libamdhip64             — HIP runtime ("cudart"-thin)
                    //   libamd_comgr            — code-object JIT ("ptxjit")
                    //   libdrm_amdgpu/libdrm    — render-node GEM path
                    //   libelf/libnuma          — .so deps slim images lack
                    // AMD's own container runtime mounts the whole host ROCm
                    // tree; this subset is strictly smaller. NB: an image that
                    // already carries a ROCm stack (rocm/*, rocm/pytorch) gets
                    // these mounts SHADOWING its image copies — keep host ROCm
                    // >= image ROCm or strip the mounts from a --dry-run argv
                    // (docs/runbook.md, AMD failure map).
                    let mut seen: Vec<String> = vec![];
                    // Merge-usr hosts (Ubuntu/Debian: /lib -> /usr/lib) would
                    // mount the same lib twice via different paths — skip
                    // libdirs that canonicalize to an already-scanned one.
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
                                bind_ro(&mut cmd, &f, &f, &mut seen);
                                // Cross-distro loader path (same fix as the
                                // NVIDIA libpath bug): an identity bind in
                                // /usr/lib64 or /opt/rocm is invisible to a
                                // Debian-family image's dynamic linker —
                                // mirror into the multiarch dir.
                                if libdir != "/usr/lib/x86_64-linux-gnu" {
                                    if let Some(name) = f.rsplit('/').next() {
                                        if !name.is_empty() {
                                            bind_ro(
                                                &mut cmd,
                                                &f,
                                                &format!("/usr/lib/x86_64-linux-gnu/{}", name),
                                                &mut seen,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Operator tools from the host (compiled ones run as-is;
                    // rocm-smi AND amd-smi are python scripts in recent ROCm
                    // and may need their module tree inside the image — the
                    // definitive attach check is rocminfo / tests/amd-smoke.py,
                    // mirroring nvidia-smi's role).
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
                                    bind_ro(&mut cmd, tool, &format!("/usr/bin/{}", name), &mut seen);
                                }
                            }
                        }
                    }
                }
                // Custom accelerators: raw nodes only — their userspace is
                // image-borne by design (docs/gemmini-case-study-plan.md and
                // the runbook); nothing NVIDIA/AMD-specific is injected.
                _ => {}
            }
        } else if acc.backend == crate::probe::Backend::Nvidia {
            // No device nodes visible (WSL / restricted namespace): the ONLY
            // docker path is --gpus all, which needs the toolkit.
            if nvidia_toolkit_present() {
                cmd.arg("--gpus").arg("all");
            } else {
                return Err(
                    "no /dev/nvidia* nodes visible to accelrun and NVIDIA Container Toolkit not installed. \
                     On WSL this is unavoidable (docker needs the toolkit); on bare metal check that the \
                     nvidia driver loaded: `ls /dev/nvidia*`."
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
    // The --cdi path carries the same asymmetry via cgroupRules (cdi.rs).
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
            custom_accel_id: None,
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
}
