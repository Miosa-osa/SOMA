//! Publishing a captured snapshot: the objects on disk, and the identity that names them.
//!
//! A generation id is written once and never rewritten. Two captures racing on one entry both
//! try to link it, exactly one wins, and the loser reads back what is there rather than
//! replacing it, so a directory can never end up naming two Generations.

use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write as _,
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
};

use soma_generation::{
    ArtifactDescriptor, ArtifactRole, CompilerProfile, PublishedCandidate, Sha256Digest,
    SnapshotSource, certify_candidate, install_snapshot, promote_candidate,
};
use soma_kvm::x86_64::CaptureOutcome;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn publish_generation_id(entry: &Path, identity: &str) -> Result<(), Box<dyn Error>> {
    let path = entry.join("generation.id");
    if path.exists() {
        return existing_generation_id(&path, identity);
    }
    let temporary = entry.join(format!(
        ".generation.id.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true).mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(identity.as_bytes())?;
    file.sync_all()?;
    match fs::hard_link(&temporary, &path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temporary)?;
            return existing_generation_id(&path, identity);
        }
        Err(error) => {
            let _ignored = fs::remove_file(&temporary);
            return Err(error.into());
        }
    }
    fs::remove_file(&temporary)?;
    File::open(entry)?.sync_all()?;
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn existing_generation_id(path: &Path, identity: &str) -> Result<(), Box<dyn Error>> {
    let existing = fs::read_to_string(path)?;
    if existing == identity {
        Ok(())
    } else {
        Err("generation.id already names another Generation".into())
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn descriptor(
    role: ArtifactRole,
    digest: soma_kvm::snapshot::Digest,
    size: u64,
) -> ArtifactDescriptor {
    ArtifactDescriptor {
        role,
        digest: Sha256Digest::from_bytes(*digest.as_bytes()),
        size,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn install_and_publish(
    entry: &Path,
    store: &Path,
    candidate: &PublishedCandidate,
    outcome: &CaptureOutcome,
) -> Result<(), Box<dyn Error>> {
    let mut memory = File::open(outcome.paths.memory())?;
    let mut overlay = File::open(outcome.paths.overlay())?;
    let mut state = File::open(outcome.paths.state())?;
    let binding = install_snapshot(
        store,
        SnapshotSource::new(
            &mut memory,
            descriptor(
                ArtifactRole::MemorySnapshot,
                outcome.memory_digest,
                outcome.memory_bytes,
            ),
        ),
        SnapshotSource::new(
            &mut overlay,
            descriptor(
                ArtifactRole::OverlaySnapshot,
                outcome.overlay_digest,
                outcome.overlay_bytes,
            ),
        ),
        SnapshotSource::new(
            &mut state,
            descriptor(
                ArtifactRole::StateManifest,
                outcome.state_digest,
                outcome.state_bytes,
            ),
        ),
    )?;
    let certification = certify_candidate(store, candidate, &CompilerProfile::v1(), binding)?;
    let generation = promote_candidate(store, candidate, &certification)?;
    publish_generation_id(entry, generation.id.as_str())?;
    println!(
        "captured {}\n  generation {}\n  memory {} bytes\n  overlay {} bytes\n  state {} bytes",
        outcome.paths.memory().parent().unwrap_or(entry).display(),
        generation.id.as_str(),
        outcome.memory_bytes,
        outcome.overlay_bytes,
        outcome.state_bytes,
    );
    Ok(())
}
