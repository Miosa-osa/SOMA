//! Turning one prepared Generation into everything a machine needs to boot.

use std::os::fd::{AsFd, AsRawFd};
use std::path::PathBuf;

use soma::{BackendFailureKind, InstanceId};
use soma_guest::LaunchNetwork;
use soma_storage::CloneError;

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

/// Gives this Instance a private writable head over the sterile overlay template.
///
/// On a reflink filesystem the head is a `FICLONE` of the template, which shares its extents
/// until they are written and so costs neither the time nor the space of a copy. Where the
/// filesystem cannot reflink, the bytes are copied instead: the head must still be private, so
/// the fallback is slower rather than absent, and the two paths produce the same head.
fn private_head(
    store: &std::path::Path,
    descriptor: &soma_generation::ArtifactDescriptor,
    instance: &InstanceId,
) -> Result<std::fs::File, BackendFailureKind> {
    let template = soma_generation::open_artifact(store, descriptor)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let directory = head_directory()?;
    // A head name is lowercase, digits, and hyphen only, so the Instance identity is used as
    // it is rather than given a suffix the validator would reject.
    let name = soma_storage::HeadName::new(instance.as_str().to_ascii_lowercase())
        .map_err(|_| BackendFailureKind::Unavailable)?;
    match soma_storage::clone_head(template.as_fd(), directory.as_fd(), &name) {
        Ok(head) => {
            // The head is unlinked immediately: the open descriptor keeps it alive for the
            // machine, so nothing on the filesystem outlives the sandbox that owns it.
            unlink_quietly(&directory, name.as_str());
            Ok(std::fs::File::from(head.into_fd()))
        }
        // Only the absence of the capability falls back. Every other failure is a real one and
        // must not be hidden behind a slow path that would succeed and look identical.
        Err(CloneError::ReflinkUnsupported | CloneError::CrossDevice) => {
            copied_head(template, &directory, name.as_str())
        }
        Err(_) => Err(BackendFailureKind::Unavailable),
    }
}

/// The directory private heads are created in.
///
/// An operator names a reflink-capable directory to get the fast path; the default is the
/// ordinary temporary directory, which usually cannot reflink and therefore copies.
fn head_directory() -> Result<std::fs::File, BackendFailureKind> {
    let path = std::env::var_os("SOMA_HEAD_DIR").map_or_else(
        || std::env::temp_dir().join("soma-kvm-heads"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&path).map_err(|_| BackendFailureKind::Unavailable)?;
    std::fs::File::open(&path).map_err(|_| BackendFailureKind::Unavailable)
}

/// The fallback head: the same private bytes, copied rather than shared.
fn copied_head(
    mut template: std::fs::File,
    directory: &std::fs::File,
    name: &str,
) -> Result<std::fs::File, BackendFailureKind> {
    let path = directory_path(directory)?.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).read(true).write(true);
    let mut head = options
        .open(&path)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    std::io::copy(&mut template, &mut head).map_err(|_| BackendFailureKind::Unavailable)?;
    let _ignored = std::fs::remove_file(&path);
    Ok(head)
}

fn directory_path(directory: &std::fs::File) -> Result<PathBuf, BackendFailureKind> {
    let fd = directory.as_raw_fd();
    std::fs::read_link(format!("/proc/self/fd/{fd}")).map_err(|_| BackendFailureKind::Unavailable)
}

fn unlink_quietly(directory: &std::fs::File, name: &str) {
    if let Ok(path) = directory_path(directory) {
        let _ignored = std::fs::remove_file(path.join(name));
    }
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
