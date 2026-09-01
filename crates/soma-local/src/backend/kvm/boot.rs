//! Turning one prepared Generation into everything a machine needs to boot.

use std::num::NonZeroUsize;
use std::os::fd::{AsFd, AsRawFd};
use std::path::PathBuf;

use soma::{BackendFailureKind, InstanceId};
use soma_guest::{LaunchNetwork, SecretFile};
use soma_kvm::x86_64::{Hypervisor, SandboxDisks, SnapshotObjects, SnapshotPaths};
use soma_storage::CloneError;

use super::identity::{LaunchIdentity, candidate_bytes, now_unix_nanos};
use super::prepared::PreparedGeneration;
use soma_vmm::sandbox::{Boot, ColdBootInputs, Network, Source, cold_boot_config};

/// Opens the prepared artifacts and gives this Instance its own writable overlay head.
///
/// The overlay template in the store is sterile and shared, so it is never opened writable:
/// each Instance receives a private copy, and two Instances of one Generation therefore
/// cannot observe each other's writes.
pub(super) fn boot_for(
    prepared: &PreparedGeneration,
    memory_mib: u64,
    identity: LaunchIdentity,
    network: Network,
    secrets: Vec<SecretFile>,
) -> Result<Boot, BackendFailureKind> {
    let instance = &InstanceId::new(hex(identity.instance))
        .map_err(|_| BackendFailureKind::WorkloadRejected)?;
    let manifest = &prepared.manifest;
    let open = |descriptor| {
        soma_generation::open_artifact(&prepared.store, descriptor)
            .map_err(|_| BackendFailureKind::Unavailable)
    };
    let kernel = open(&manifest.kernel.descriptor)?;
    let initramfs = open(&manifest.initramfs.descriptor)?;
    let root = open(&manifest.root.descriptor)?;
    // A Generation that declared no writable storage published no sterile template, and the
    // whole point of it is that no head is cloned on the request path: the clone of a private
    // head is the largest and most variable cost between admission and a launched machine.
    let devices = manifest.device_set();
    let template = if devices.overlay() {
        Some(
            manifest
                .overlay
                .templates
                .first()
                .ok_or(BackendFailureKind::Unavailable)?,
        )
    } else {
        None
    };
    // A prepared entry may carry a snapshot taken once for the whole Generation. When it does,
    // this launch resumes that machine instead of booting a kernel, which is the difference
    // between hundreds of milliseconds and tens on the request path.
    let snapshot = super::claim::snapshot_dir(prepared);
    let guest_cid = identity.guest_cid;
    let source = if let Some(snapshot) = snapshot {
        {
            // The restore clones its own head from the snapshot's sterile overlay template, not
            // from the Candidate's, because the captured machine has already written to it.
            let overlay = devices
                .overlay()
                .then(|| private_head_from(&snapshot.join("overlay.raw"), instance))
                .transpose()?;
            let objects = SnapshotObjects::open(&SnapshotPaths::new(snapshot))
                .map_err(|_| BackendFailureKind::Unavailable)?;
            Source::Restore {
                objects,
                hypervisor: Hypervisor::Device,
                disks: SandboxDisks { root, overlay },
                devices,
                memory_bytes: memory_mib * MIB,
            }
        }
    } else {
        {
            let overlay = template
                .map(|template| private_head(&prepared.store, &template.descriptor, instance))
                .transpose()?;
            Source::ColdBoot(cold_boot_config(ColdBootInputs {
                kernel,
                initramfs,
                root,
                overlay,
                ram_bytes: memory_mib * MIB,
                guest_cid,
                devices,
            }))
        }
    };
    Ok(Boot {
        source,
        generation: candidate_bytes(&prepared.id)?,
        instance: identity.instance,
        operation: identity.operation,
        guest_cid,
        network,
        secrets,
    })
}

/// The Instance identity as the lowercase hexadecimal its portable form is written in.
fn hex(instance: [u8; 16]) -> String {
    use std::fmt::Write as _;
    instance
        .iter()
        .fold(String::with_capacity(32), |mut out, byte| {
            let _ignored = write!(out, "{byte:02x}");
            out
        })
}

/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// Gives this Instance a private writable head over an explicit template file.
///
/// The snapshot carries its own sterile overlay template, quiesced at the capture point, so a
/// restored Instance must clone that rather than the Candidate's untouched one.
pub(super) fn private_head_from(
    template_path: &std::path::Path,
    instance: &InstanceId,
) -> Result<std::fs::File, BackendFailureKind> {
    let template =
        std::fs::File::open(template_path).map_err(|_| BackendFailureKind::Unavailable)?;
    clone_or_copy(template, None, instance)
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
    clone_or_copy(template, Some(*descriptor.digest.as_bytes()), instance)
}

/// Clones one open template into a private head for `instance`, reflinking where it can.
///
/// On a reflink filesystem the head shares the template's extents until it is written, so it
/// costs neither the time nor the space of a copy. Where the filesystem cannot reflink the bytes
/// are copied instead: the head must still be private, so the fallback is slower rather than
/// absent, and both paths produce the same head.
fn clone_or_copy(
    template: std::fs::File,
    certified_digest: Option<[u8; 32]>,
    instance: &InstanceId,
) -> Result<std::fs::File, BackendFailureKind> {
    let directory = head_directory()?;
    let template = match certified_digest {
        Some(digest) => fanned(template, digest),
        None => template,
    };
    // A head name is lowercase, digits, and hyphen only, so the Instance identity is used as
    // it is rather than given a suffix the validator would reject.
    let name = soma_storage::HeadName::new(instance.as_str().to_ascii_lowercase())
        .map_err(|_| BackendFailureKind::Unavailable)?;
    match soma_storage::clone_head(
        template.as_fd(),
        directory.as_fd(),
        &name,
        // The head is unlinked before this function returns and is read only through the
        // descriptor handed to the machine, so there is nothing for a sync to publish and no
        // crash it should survive. Syncing it anyway pushed the filesystem log once per launch
        // and was almost the whole cost of giving an Instance its overlay.
        soma_storage::Durability::Ephemeral,
    ) {
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

/// The root private heads are created under.
///
/// An operator names a reflink-capable directory to get the fast path; the default is the
/// ordinary temporary directory, which usually cannot reflink and therefore copies.
fn head_root() -> PathBuf {
    std::env::var_os("SOMA_HEAD_DIR").map_or_else(
        || std::env::temp_dir().join("soma-kvm-heads"),
        PathBuf::from,
    )
}

/// A shard of the head root, because creating and unlinking a head takes the directory's inode
/// lock and a cohort of a hundred launches otherwise queues on one directory.
fn head_directory() -> Result<std::fs::File, BackendFailureKind> {
    soma_storage::open_shard(
        &head_root(),
        count("SOMA_HEAD_SHARDS", soma_storage::DEFAULT_HEAD_SHARDS),
    )
    .map_err(|_| BackendFailureKind::Unavailable)
}

/// One independent physical copy of the template, when a warmed fan holds one.
///
/// Cloning from one template serializes every launch of a cohort on the refcount records of
/// that template's extents. A fan is warmed off the launch path; where it is absent this hands
/// back the template itself and the launch pays what it always paid.
fn fanned(template: std::fs::File, certified_digest: [u8; 32]) -> std::fs::File {
    let root = std::env::var_os("SOMA_TEMPLATE_FAN_DIR")
        .map_or_else(|| head_root().join("fan"), PathBuf::from);
    let copies = count(
        "SOMA_TEMPLATE_COPIES",
        soma_storage::DEFAULT_TEMPLATE_COPIES,
    );
    soma_storage::open_replica(&template, certified_digest, &root, copies).unwrap_or(template)
}

/// A positive count named by the environment, or the default this build ships.
fn count(variable: &str, fallback: usize) -> NonZeroUsize {
    std::env::var(variable)
        .ok()
        .and_then(|value| value.parse().ok())
        .and_then(NonZeroUsize::new)
        .unwrap_or_else(|| NonZeroUsize::new(fallback).unwrap_or(NonZeroUsize::MIN))
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
pub(super) fn link_down_network(guest_cid: u32) -> Result<LaunchNetwork, BackendFailureKind> {
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

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
