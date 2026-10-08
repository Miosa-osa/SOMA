//! Turning one capture outcome into a published, certified Generation.
//!
//! The ready identity is published last, so a prepared entry is either a Candidate or a
//! complete Generation, never a partially promoted mixture. Everything before that point is
//! reversible; the identity is the one step that makes the entry launchable.

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

pub(super) fn candidate_bytes(hex: &str) -> Result<[u8; 32], Box<dyn Error>> {
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
pub(super) fn source_head(template: &mut File, path: &Path) -> Result<File, Box<dyn Error>> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).read(true).write(true);
    let mut head = options.open(path)?;
    std::io::copy(template, &mut head)?;
    Ok(head)
}

/// Publishes the ready identity last so a prepared entry is either a Candidate or a complete
/// Generation, never a partially promoted mixture.
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
