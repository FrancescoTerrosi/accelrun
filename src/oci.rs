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
            cmd.arg("-e").arg("NVIDIA_VISIBLE_DEVICES=all")
               .arg("-e").arg("NVIDIA_DRIVER_CAPABILITIES=compute,utility");
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
    // docker's -m wants raw bytes (or lowercase "16g", NOT "16Gi") — pass bytes
    cmd.arg("--memory").arg(mem_bytes.to_string()).arg(&args.image);
    cmd.args(&args.cmd);
    if args.dry_run {
        let mut parts: Vec<String> = vec![cmd.get_program().to_string_lossy().into_owned()];
        parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
        println!("[accelrun] DRY-RUN: {}", parts.join(" "));
        return Ok(());
    }
    println!(
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
