//! Turning one prepared Generation into everything a machine needs to boot.

use std::os::fd::{AsFd, AsRawFd};
use std::path::PathBuf;

use soma::{BackendFailureKind, InstanceId};
use soma_guest::LaunchNetwork;
use soma_kvm::x86_64::SandboxDisks;
use soma_storage::CloneError;

use super::prepared::PreparedGeneration;
use super::session::{Boot, FIRST_GUEST_CID, Source};

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
    // A prepared entry may carry a snapshot taken once for the whole Generation. When it does,
    // this launch resumes that machine instead of booting a kernel, which is the difference
    // between hundreds of milliseconds and tens on the request path.
    let snapshot = prepared.store.parent().map(|entry| entry.join("snapshot"));
    let guest_cid = guest_cid_for(instance)?;
    let source =
        if let Some(snapshot) = snapshot.filter(|path| path.join("state.somasnap").is_file()) {
            {
                // The restore clones its own head from the snapshot's sterile overlay template, not
                // from the Candidate's, because the captured machine has already written to it.
                let overlay = private_head_from(&snapshot.join("overlay.raw"), instance)?;
                Source::Restore {
                    snapshot,
                    disks: SandboxDisks { root, overlay },
                    memory_bytes: memory_mib * MIB,
                }
            }
        } else {
            {
                let overlay = private_head(&prepared.store, &template.descriptor, instance)?;
                Source::ColdBoot(super::worker::config(
                    kernel,
                    initramfs,
                    root,
                    overlay,
                    memory_mib * MIB,
                    guest_cid,
                ))
            }
        };
    Ok(Boot {
        source,
        generation: candidate_bytes(&prepared.id)?,
        instance: instance_bytes(instance)?,
        operation: fresh16(),
        guest_cid,
        network: link_down_network(guest_cid)?,
    })
}

/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// Gives this Instance a private writable head over an explicit template file.
///
/// The snapshot carries its own sterile overlay template, quiesced at the capture point, so a
/// restored Instance must clone that rather than the Candidate's untouched one.
fn private_head_from(
    template_path: &std::path::Path,
    instance: &InstanceId,
) -> Result<std::fs::File, BackendFailureKind> {
    let template =
        std::fs::File::open(template_path).map_err(|_| BackendFailureKind::Unavailable)?;
    clone_or_copy(template, instance)
}

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
    clone_or_copy(template, instance)
}

/// Clones one open template into a private head for `instance`, reflinking where it can.
///
/// On a reflink filesystem the head shares the template's extents until it is written, so it
/// costs neither the time nor the space of a copy. Where the filesystem cannot reflink the bytes
/// are copied instead: the head must still be private, so the fallback is slower rather than
/// absent, and both paths produce the same head.
fn clone_or_copy(
    template: std::fs::File,
    instance: &InstanceId,
) -> Result<std::fs::File, BackendFailureKind> {
    let directory = head_directory()?;
    // A head name is lowercase, digits, and hyphen only, so the Instance identity is used as
    // it is rather than given a suffix the validator would reject.
    let name = soma_storage::HeadName::new(instance.as_str().to_ascii_lowercase())
        .map_err(|_| BackendFailureKind::Unavailable)?;
    match soma_storage::clone_head(template.as_fd(), directory.as_fd(), &name) {
        Ok(head) => {
            // The head is unlinked immediately: the open descriptor keeps it alive for the
            // machine, so nothing on the filesystem outlives the sandbox that owns it. A head
            // that could not be unlinked would outlive its sandbox with no owner and no record,
            // so it fails the Launch rather than being left behind.
            unlink(&directory, name.as_str())?;
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
    // A failed copy must not leave a partly written head named on the filesystem, so the
    // destination is removed before the failure is returned.
    if std::io::copy(&mut template, &mut head).is_err() {
        let _ignored = std::fs::remove_file(&path);
        return Err(BackendFailureKind::Unavailable);
    }
    std::fs::remove_file(&path).map_err(|_| BackendFailureKind::Unavailable)?;
    Ok(head)
}

fn directory_path(directory: &std::fs::File) -> Result<PathBuf, BackendFailureKind> {
    let fd = directory.as_raw_fd();
    std::fs::read_link(format!("/proc/self/fd/{fd}")).map_err(|_| BackendFailureKind::Unavailable)
}

/// Removes one head by name, reporting failure rather than ignoring it.
fn unlink(directory: &std::fs::File, name: &str) -> Result<(), BackendFailureKind> {
    let path = directory_path(directory)?.join(name);
    std::fs::remove_file(path).map_err(|_| BackendFailureKind::Unavailable)
}

/// The link-down placeholder network every guest is given today.
///
/// The addresses are fixed because nothing routes them: the device exists so the guest's repair
/// step has one to configure, and no packet leaves the machine.
///
/// The context identifier is not fixed. The guest agent checks the identifier its own vsock
/// device reports against the one the launch page names, and refuses the session when they
/// disagree, which is what binds the transport the session runs over to this Instance's
/// authority. So this must be given the same identifier the machine was built with rather than
/// a constant: a launch page naming a different one leaves a correctly built machine unable to
/// form a session at all.
fn link_down_network(guest_cid: u32) -> Result<LaunchNetwork, BackendFailureKind> {
    LaunchNetwork::new(
        guest_cid,
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

/// The exact sixteen bytes the launch page carries for this Instance.
///
/// The guest authenticates an Instance identity, and the receipt reports one. If those were
/// allowed to differ, the authenticated session would prove one Instance while the public
/// evidence described another, and no reader could tell. This is the only conversion between
/// them, and it is exact rather than derived: a `InstanceId` is thirty-two lowercase hexadecimal
/// characters, which is these sixteen bytes written out.
fn instance_bytes(instance: &InstanceId) -> Result<[u8; 16], BackendFailureKind> {
    let hex = instance.as_str();
    if hex.len() != 32 {
        return Err(BackendFailureKind::WorkloadRejected);
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        bytes[index] =
            u8::from_str_radix(text, 16).map_err(|_| BackendFailureKind::WorkloadRejected)?;
    }
    Ok(bytes)
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

/// The vsock context identifier this Instance takes.
///
/// Context identifiers are host global, so two concurrent sandboxes sharing one would contend for
/// the same endpoint. One command line invocation serves one sandbox and cannot see the others,
/// so there is no counter to draw from: the identifier is derived from the Instance identity,
/// which is already unique per sandbox. Zero, one, and two are reserved by the kernel, so the
/// derived value is folded into the range above them.
fn guest_cid_for(instance: &InstanceId) -> Result<u32, BackendFailureKind> {
    let bytes = instance_bytes(instance)?;
    let derived = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    // `u32::MAX` is reserved as the "any" identifier, so the usable span ends one below it.
    let span = u32::MAX - FIRST_GUEST_CID;
    Ok(FIRST_GUEST_CID + (derived % span))
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
