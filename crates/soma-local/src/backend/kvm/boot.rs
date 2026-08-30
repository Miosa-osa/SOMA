//! Turning one prepared Generation into everything a machine needs to boot.

use soma::{BackendFailureKind, InstanceId};
use soma_guest::LaunchNetwork;

use super::prepared::PreparedGeneration;
use super::session::Boot;

/// Opens the prepared artifacts and gives this Instance its own writable overlay head.
///
/// The overlay template in the store is sterile and shared, so it is never opened writable:
/// each Instance receives a private copy, and two Instances of one Generation therefore
/// cannot observe each other's writes.
pub(super) fn boot_for(
    prepared: &PreparedGeneration,
    memory_mib: u64,
    instance: &InstanceId,
) -> Result<Boot, BackendFailureKind> {
    let manifest = &prepared.manifest;
    let open = |descriptor| {
        soma_generation::open_artifact(&prepared.store, descriptor)
            .map_err(|_| BackendFailureKind::Unavailable)
    };
    let kernel = open(&manifest.kernel.descriptor)?;
    let initramfs = open(&manifest.initramfs.descriptor)?;
    let root = open(&manifest.root.descriptor)?;
    let template = manifest
        .overlay
        .templates
        .first()
        .ok_or(BackendFailureKind::Unavailable)?;
    let overlay = private_head(&prepared.store, &template.descriptor, instance)?;
    Ok(Boot {
        config: super::worker::config(kernel, initramfs, root, overlay, memory_mib * MIB),
        generation: candidate_bytes(&prepared.id)?,
        instance: fresh16(),
        machine: fresh16(),
        network: link_down_network()?,
    })
}

/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// Copies the sterile overlay template into a head only this Instance can write.
fn private_head(
    store: &std::path::Path,
    descriptor: &soma_generation::ArtifactDescriptor,
    instance: &InstanceId,
) -> Result<std::fs::File, BackendFailureKind> {
    let mut template = soma_generation::open_artifact(store, descriptor)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let directory = std::env::temp_dir().join("soma-kvm-heads");
    std::fs::create_dir_all(&directory).map_err(|_| BackendFailureKind::Unavailable)?;
    let path = directory.join(format!("{}.ext4", instance.as_str()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    options.truncate(true);
    let mut head = options
        .open(&path)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    std::io::copy(&mut template, &mut head).map_err(|_| BackendFailureKind::Unavailable)?;
    // The head is unlinked immediately: the open descriptor keeps it alive for the machine, so
    // nothing on the filesystem outlives the sandbox that owns it.
    let _ignored = std::fs::remove_file(&path);
    Ok(head)
}

/// The link-down placeholder network every guest is given today.
///
/// The addresses are fixed because nothing routes them: the device exists so the guest's repair
/// step has one to configure, and no packet leaves the machine.
fn link_down_network() -> Result<LaunchNetwork, BackendFailureKind> {
    LaunchNetwork::new(
        3,
        1,
        [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01],
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        now_unix_nanos(),
    )
    .map_err(|_| BackendFailureKind::Unavailable)
}

fn now_unix_nanos() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )
    .unwrap_or(0)
}

/// Sixteen fresh bytes for one identity.
fn fresh16() -> [u8; 16] {
    use std::io::Read as _;
    let mut bytes = [0_u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ignored = file.read_exact(&mut bytes);
    }
    bytes
}

/// The Candidate identity as the thirty-two bytes the launch page binds.
///
/// The identity is carried as its canonical `sha256:` form, and the launch page binds raw bytes,
/// so the hex is decoded rather than re-hashed: re-hashing would bind a different value.
fn candidate_bytes(id: &soma_generation::CandidateId) -> Result<[u8; 32], BackendFailureKind> {
    let hex = id
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(BackendFailureKind::Unavailable)?;
    let mut bytes = [0_u8; 32];
    if hex.len() != 64 {
        return Err(BackendFailureKind::Unavailable);
    }
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| BackendFailureKind::Unavailable)?;
        bytes[index] = u8::from_str_radix(text, 16).map_err(|_| BackendFailureKind::Unavailable)?;
    }
    Ok(bytes)
}
