//! Boots the source machine and captures it at the guest agent's repair point.
//!
//! The source machine is booted with **no launch page**, so it reaches the repair point with no
//! Instance identity, no session, and no key anywhere in guest memory. That is what makes the
//! captured object safe to share across every Instance restored from it.

use std::{
    error::Error,
    fs::{self, File},
    path::Path,
    time::{Duration, Instant},
};

use soma_generation::{
    ArtifactDescriptor, ArtifactRole, CandidateId, PublishedCandidate, Sha256Digest,
    generation_manifest::decode_candidate, open_artifact,
};
use soma_kvm::x86_64::{
    CaptureRequest, DeviceIdentity, SandboxConfig, SandboxDisks, SandboxMachine, capture,
};

use super::publish::{candidate_bytes, install_and_publish, source_head};
use super::{CAPTURE_CID, GUEST_MAC, MIB, PAUSE_GRACE, REPAIR_POINT_DEADLINE, REPAIR_POINT_LINE};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn run(entry: &Path, memory_mib: u64) -> Result<(), Box<dyn Error>> {
    let store = entry.join("store");
    let bytes = fs::read(entry.join("candidate.somacan"))?;
    let manifest = decode_candidate(&bytes).map_err(|error| format!("{error:?}"))?;
    let candidate = PublishedCandidate {
        id: CandidateId::of(&bytes),
        descriptor: ArtifactDescriptor {
            role: ArtifactRole::GenerationCandidate,
            digest: Sha256Digest::of(&bytes),
            size: u64::try_from(bytes.len())?,
        },
        manifest: manifest.clone(),
    };
    let candidate_id = candidate_bytes(candidate.id.as_str())?;

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

    // The transcript up to the repair point is the evidence of what ran before the capture,
    // such as a declared capture warm plan, so it is kept rather than discarded on success.
    fs::write(
        entry.join("capture-console.log"),
        String::from_utf8_lossy(&evidence.serial).as_bytes(),
    )?;
    install_and_publish(entry, &store, &candidate, &outcome)
}
