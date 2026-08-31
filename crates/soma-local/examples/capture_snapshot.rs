//! Captures one prepared entry's snapshot so later launches restore instead of cold booting.
//!
//! Cold booting a Generation costs hundreds of milliseconds on the request path. A snapshot taken
//! once, at the guest agent's disconnected repair point, lets every later launch resume a machine
//! that is already through kernel boot and userspace init.
//!
//! The source machine is booted with **no launch page**, so it reaches the repair point with no
//! Instance identity, no session, and no key anywhere in guest memory. That is what makes the
//! captured object safe to share across every Instance restored from it.
//!
//! The snapshot is written to `<entry>/snapshot/`, beside the `store/` the Candidate describes.
//!
//! Usage:
//!
//! ```text
//! capture_snapshot <prepared-entry> [memory_mib]
//! ```

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{
    error::Error,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use soma_generation::{generation_manifest::decode_candidate, open_artifact};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use soma_kvm::x86_64::{
    CaptureRequest, DeviceIdentity, SandboxConfig, SandboxDisks, SandboxMachine, capture,
};

/// The console line the pinned agent prints when it parks awaiting launch material.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const REPAIR_POINT_LINE: &[u8] = b"soma-guest-agent: awaiting launch material";
/// The context identifier the source machine holds; every restore is given a fresh one.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const CAPTURE_CID: u32 = 3;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const GUEST_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const MIB: u64 = 1024 * 1024;
/// How long the source machine has to announce its repair point.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const REPAIR_POINT_DEADLINE: Duration = Duration::from_secs(120);
/// How long the vCPU has to leave `KVM_RUN` once it is kicked.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PAUSE_GRACE: Duration = Duration::from_secs(10);

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn candidate_bytes(hex: &str) -> Result<[u8; 32], Box<dyn Error>> {
    let hex = hex
        .strip_prefix("sha256:")
        .ok_or("candidate id is not sha256")?;
    if hex.len() != 64 {
        return Err("candidate id is not 32 bytes".into());
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        bytes[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(bytes)
}

/// Copies the sterile overlay template into the writable head the source machine boots with.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn source_head(template: &mut File, path: &Path) -> Result<File, Box<dyn Error>> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).read(true).write(true);
    let mut head = options.open(path)?;
    std::io::copy(template, &mut head)?;
    Ok(head)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run(entry: &Path, memory_mib: u64) -> Result<(), Box<dyn Error>> {
    let store = entry.join("store");
    let bytes = fs::read(entry.join("candidate.somacan"))?;
    let manifest = decode_candidate(&bytes).map_err(|error| format!("{error:?}"))?;
    let candidate_id = candidate_bytes(soma_generation::CandidateId::of(&bytes).as_str())?;

    let kernel = open_artifact(&store, &manifest.kernel.descriptor)
        .map_err(|error| format!("kernel: {error:?}"))?;
    let initramfs = open_artifact(&store, &manifest.initramfs.descriptor)
        .map_err(|error| format!("initramfs: {error:?}"))?;
    let mut root = open_artifact(&store, &manifest.root.descriptor)
        .map_err(|error| format!("root: {error:?}"))?;
    // The source machine is built as exactly the machine the Candidate declares. A Candidate
    // with no writable storage has no template to open, no head to seed, and publishes no
    // `overlay.raw`, so every Instance restored from its snapshot clones nothing.
    let devices = manifest.device_set();
    let mut template = if devices.overlay() {
        let template_descriptor = &manifest
            .overlay
            .templates
            .first()
            .ok_or("the Candidate declares writable storage but no overlay template")?
            .descriptor;
        Some(
            open_artifact(&store, template_descriptor)
                .map_err(|error| format!("overlay template: {error:?}"))?,
        )
    } else {
        None
    };

    let snapshot = entry.join("snapshot");
    if snapshot.exists() {
        return Err(format!(
            "{} already exists; remove it to recapture",
            snapshot.display()
        )
        .into());
    }
    // The capture writes staging objects inside this directory; it does not create it.
    fs::create_dir_all(&snapshot)?;
    let head_path = entry.join("capture-head.ext4");
    let mut head = template
        .as_mut()
        .map(|template| source_head(template, &head_path))
        .transpose()?;
    // The agent warms the workload runtime itself before it parks, so the runtime's pages are
    // resident when the capture records guest memory. Nothing is seeded into the overlay here:
    // the agent requires a sterile upper layer and refuses to boot if anything is placed in it.

    let config = SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks {
            root: open_artifact(&store, &manifest.root.descriptor)
                .map_err(|error| format!("root: {error:?}"))?,
            overlay: head.as_ref().map(File::try_clone).transpose()?,
        },
        identity: DeviceIdentity {
            guest_cid: CAPTURE_CID,
            guest_mac: GUEST_MAC,
        },
        ram_bytes: memory_mib * MIB,
        devices,
    };

    let mut sandbox = SandboxMachine::create(config).map_err(|error| format!("create: {error}"))?;
    sandbox.watch_console(REPAIR_POINT_LINE);
    // Deliberately no launch page: the source must reach its repair point carrying no Instance
    // identity, no session, and no key, because every restore shares these captured bytes.
    sandbox.start().map_err(|error| format!("start: {error}"))?;

    let started = Instant::now();
    let outcome = capture(
        &mut sandbox,
        CaptureRequest {
            paths: soma_kvm::x86_64::SnapshotPaths::new(snapshot.clone()),
            candidate_id,
            root: &mut root,
            overlay: head.as_mut(),
            repair_point_line: REPAIR_POINT_LINE.to_vec(),
            grace: PAUSE_GRACE,
        },
        started + REPAIR_POINT_DEADLINE,
    );
    let evidence = sandbox.finish(Duration::from_secs(10));
    let _ignored = fs::remove_file(&head_path);

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let console = String::from_utf8_lossy(&evidence.serial);
            let tail: Vec<&str> = console.lines().rev().take(12).collect();
            return Err(format!(
                "capture failed: {error:?}\nconsole tail:\n  {}",
                tail.into_iter().rev().collect::<Vec<_>>().join("\n  ")
            )
            .into());
        }
    };

    println!(
        "captured {}\n  memory {} bytes\n  overlay {} bytes\n  state {} bytes",
        snapshot.display(),
        outcome.memory_bytes,
        outcome.overlay_bytes,
        outcome.state_bytes,
    );
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() || arguments.len() > 2 {
        eprintln!("usage: capture_snapshot <prepared-entry> [memory_mib]");
        std::process::exit(2);
    }
    let entry = PathBuf::from(&arguments[0]);
    let memory_mib = arguments
        .get(1)
        .map_or(Ok(1024), |value| value.parse::<u64>())
        .unwrap_or(1024);
    if let Err(error) = run(&entry, memory_mib) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("capture_snapshot requires Linux x86_64 with KVM");
    std::process::exit(2);
}
