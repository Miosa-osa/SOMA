//! Captures the state of a resident machine into broker-owned, unpublished files.

use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    fs::File,
    os::unix::fs::{FileExt, MetadataExt},
    time::Duration,
};

use crate::{
    contract::MachineContract,
    snapshot::{
        Digest, Hasher,
        manifest::{CandidateId, CheckpointManifest, CheckpointManifestError, PageSize},
        memory::MemoryDescriptor,
    },
    virtio::{BlockDevice, Slot},
    x86_64::{
        cpuid::GuestMachine,
        error::MachineError,
        sandbox::{SandboxMachine, pause::PausedMachine},
    },
};

use super::{
    Artifact, SnapshotError,
    capture::manifest::{Parts, build_parts},
    device, profile, quiesce,
    vm_state::PausedVmState,
};

const CHUNK: usize = 1 << 20;

/// Unpublished, distinct private regular files created by the trusted broker.
pub struct CheckpointTargets<'a> {
    pub state: &'a File,
    pub memory: &'a File,
    pub overlay: Option<&'a File>,
    pub volume: Option<&'a File>,
}

/// The owning broker supplies the already-verified launch identity and pinned root digest.
pub struct CheckpointRequest<'a> {
    pub instance_id: [u8; 16],
    pub candidate_id: [u8; 32],
    pub root_digest: Digest,
    pub contract: MachineContract,
    pub grace: Duration,
    pub targets: CheckpointTargets<'a>,
}

#[derive(Debug)]
pub enum CheckpointError {
    Pause(MachineError),
    Capture(SnapshotError),
    Resume {
        capture: Option<SnapshotError>,
        resume: MachineError,
    },
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pause(error) => write!(formatter, "checkpoint did not confirm a pause: {error}"),
            Self::Capture(error) => write!(formatter, "{error}"),
            Self::Resume { resume, .. } => write!(
                formatter,
                "checkpoint could not resume the source: {resume}"
            ),
        }
    }
}

impl Error for CheckpointError {}

impl From<SnapshotError> for CheckpointError {
    fn from(error: SnapshotError) -> Self {
        Self::Capture(error)
    }
}

/// Pauses a running guest, captures its state into unpublished files, and resumes that guest.
///
/// Capture requires idle device queues and no plugged memory.
/// A mounted durable volume needs its own private target to preserve the same point in time.
/// The guest link is held down while state is captured and restored to its source setting before
/// the source resumes, including after a rejected capture.
/// The host must discard the staging files on error and authenticate the result before publication.
///
/// # Errors
///
/// For invalid targets or unsupported live state, returns a capture failure.
/// For a failure to resume after capture, returns a distinct failure with the resume error.
pub fn capture_checkpoint(
    sandbox: &SandboxMachine,
    request: &CheckpointRequest<'_>,
) -> Result<CheckpointManifest, CheckpointError> {
    validate_targets(&request.targets)?;
    CandidateId::new(request.candidate_id).map_err(SnapshotError::from)?;
    if request.instance_id == [0; 16] {
        return Err(SnapshotError::from(CheckpointManifestError::ZeroInstanceId).into());
    }
    if !sandbox.is_running() {
        return Err(SnapshotError::NotQuiescent("checkpoint requires a running sandbox").into());
    }
    sandbox
        .pause(request.grace)
        .map_err(CheckpointError::Pause)?;
    let link_was_up = sandbox.network_link_up();
    sandbox.set_network_link(false);
    let capture = sandbox
        .with_paused(|paused| capture_paused(paused, request, sandbox.hotplug_region()))
        .unwrap_or(Err(SnapshotError::NotQuiescent(
            "checkpoint did not reach a complete pause",
        )));
    sandbox.set_network_link(link_was_up);
    if let Err(resume) = sandbox.resume() {
        return Err(CheckpointError::Resume {
            capture: capture.err(),
            resume,
        });
    }
    let manifest = capture?;
    let encoded = manifest.encode();
    let state = request.targets.state;
    write_at(Artifact::State, state, &encoded, 0)?;
    seal(
        Artifact::Memory,
        request.targets.memory,
        manifest.header().memory.digest(),
    )?;
    for (artifact, slot, target) in [
        (Artifact::Overlay, Slot::Overlay, request.targets.overlay),
        (Artifact::Volume, Slot::Volume, request.targets.volume),
    ] {
        if let Some(target) = target {
            seal(artifact, target, block_digest(&manifest, slot)?)?;
        }
    }
    seal(Artifact::State, state, Digest::of(&encoded))?;
    Ok(manifest)
}

fn validate_targets(targets: &CheckpointTargets<'_>) -> Result<(), SnapshotError> {
    let mut inodes = BTreeSet::new();
    for (artifact, file) in [
        (Artifact::State, Some(targets.state)),
        (Artifact::Memory, Some(targets.memory)),
        (Artifact::Overlay, targets.overlay),
        (Artifact::Volume, targets.volume),
    ] {
        let Some(file) = file else {
            continue;
        };
        let meta = file
            .metadata()
            .map_err(|error| SnapshotError::io(artifact, "inspect staging file", &error))?;
        if !meta.is_file()
            || meta.len() != 0
            || meta.mode() & 0o077 != 0
            || !inodes.insert((meta.dev(), meta.ino()))
        {
            return Err(SnapshotError::NotQuiescent(
                "checkpoint targets must be empty, private, and distinct regular files",
            ));
        }
    }
    Ok(())
}

fn capture_paused(
    mut paused: PausedMachine<'_>,
    request: &CheckpointRequest<'_>,
    hotplug: Option<crate::memory_layout::HotplugRegion>,
) -> Result<CheckpointManifest, SnapshotError> {
    let bus = &mut *paused.bus;
    let has_mounted_volume = bus
        .volume()
        .is_some_and(|volume| !volume.device().is_placeholder());
    if bus.volume().is_some() != has_mounted_volume
        || has_mounted_volume != request.targets.volume.is_some()
        || bus.overlay().is_some() != request.targets.overlay.is_some()
    {
        return Err(SnapshotError::NotQuiescent(
            "the checkpoint storage targets do not cover the guest devices",
        ));
    }
    // No device work here: a retry may see idle queues, and the resumed source keeps its pending
    // notifications and interrupts even when this capture refuses.
    if bus.net().is_some_and(|net| net.device().link_up()) {
        return Err(SnapshotError::NotQuiescent(
            "checkpoint requires the guest network link down",
        ));
    }
    if let Some(mem) = bus.mem().filter(|mem| !mem.device().is_quiescent()) {
        return Err(SnapshotError::MemoryPlugged {
            plugged_bytes: mem.device().plugged_size(),
        });
    }
    // An idle command session may exist in the source. None of its host-side transport state is
    // captured. A future restore must reset the guest channel before authenticating it again.
    quiesce::prove_guest_queues_idle(bus, &paused.memory)?;
    if !request.contract.accepts_memory(paused.ram_bytes) {
        return Err(SnapshotError::NotQuiescent(
            "checkpoint RAM exceeds the machine contract",
        ));
    }
    let machine = PausedVmState::read(
        paused.kvm,
        paused.vm,
        &paused.vcpus,
        paused.ram_bytes,
        |_| Ok(()),
    )?;
    let vcpus = u16::try_from(machine.vcpus.len())
        .map_err(|_| SnapshotError::NotQuiescent("checkpoint has too many vCPUs"))?;
    let (_, cpu_template) =
        profile::cpu_template(paused.kvm, GuestMachine::new(request.contract, vcpus))?;
    let overlay = capture_block(
        Artifact::Overlay,
        bus.overlay_mut().map(|slot| slot.device_mut()),
        request.targets.overlay,
    )?;
    let volume = capture_block(
        Artifact::Volume,
        bus.volume_mut().map(|slot| slot.device_mut()),
        request.targets.volume,
    )?;
    let devices = capture_devices(bus, request, overlay, volume)?;
    let memory = copy_image(
        Artifact::Memory,
        request.targets.memory,
        paused.ram_bytes,
        |offset, bytes| {
            if paused.memory.read_image(offset, bytes) {
                Ok(())
            } else {
                Err(SnapshotError::NotQuiescent(
                    "guest RAM changed during checkpoint",
                ))
            }
        },
    )?;
    let (header, sections) = build_parts(
        request.candidate_id,
        &machine.vm,
        &machine.vcpus,
        &Parts {
            contract: request.contract,
            cpu_template,
            irqchip: &machine.irqchip,
            routing: &machine.routing,
            clock: &machine.clock,
            pit: &machine.pit,
            devices: &devices,
            device_set: bus.device_set(),
            hotplug,
            memory: MemoryDescriptor::new(memory, paused.ram_bytes, PageSize::FOUR_KIB.get())?,
        },
    )?;
    Ok(CheckpointManifest::new(
        request.instance_id,
        header,
        sections,
    )?)
}

fn capture_block(
    artifact: Artifact,
    device: Option<&mut BlockDevice>,
    target: Option<&File>,
) -> Result<Digest, SnapshotError> {
    match (device, target) {
        (None, None) => Ok(Digest::from_bytes([0; 32])),
        (Some(device), Some(file)) => {
            device
                .flush_checkpoint()
                .map_err(|_| SnapshotError::NotQuiescent("could not flush the checkpoint disk"))?;
            copy_image(
                artifact,
                file,
                device.capacity_sectors() * 512,
                |offset, bytes| match device.read_checkpoint(offset, bytes) {
                    Ok(n) if n == bytes.len() => Ok(()),
                    _ => Err(SnapshotError::NotQuiescent(
                        "could not read the checkpoint disk",
                    )),
                },
            )
        }
        _ => Err(SnapshotError::NotQuiescent(
            "checkpoint block device target mismatch",
        )),
    }
}

fn capture_devices(
    bus: &mut crate::virtio::MmioBus,
    request: &CheckpointRequest<'_>,
    overlay: Digest,
    volume: Digest,
) -> Result<Vec<(Slot, crate::snapshot::device_state::DeviceState)>, SnapshotError> {
    bus.snapshot_all()
        .into_iter()
        .map(|record| {
            let slot = record.slot;
            let digest = match slot {
                Slot::Root => request.root_digest,
                Slot::Volume => volume,
                _ => overlay,
            };
            let specific = device::specific(bus, slot, digest)
                .ok_or(SnapshotError::DeviceStateNotCanonical(slot))?;
            let state = device::canonical(slot, &record, specific, request.contract)?;
            if !device::reproduces(slot, &record, &state) {
                return Err(SnapshotError::DeviceStateNotCanonical(slot));
            }
            Ok((slot, state))
        })
        .collect()
}

fn block_digest(manifest: &CheckpointManifest, slot: Slot) -> Result<Digest, SnapshotError> {
    use crate::snapshot::device_state::{DeviceSpecific, DeviceState};

    let section = manifest
        .sections()
        .iter()
        .find(|section| Some(section.role()) == device::role(slot))
        .ok_or(SnapshotError::MissingSection("checkpoint block device"))?;
    let state = DeviceState::decode_for_slot(slot.index(), section.payload())?;
    match state.specific() {
        DeviceSpecific::Block(block) => Ok(block.image_digest),
        _ => Err(SnapshotError::DeviceStateNotCanonical(slot)),
    }
}

fn copy_image(
    artifact: Artifact,
    file: &File,
    size: u64,
    mut read: impl FnMut(u64, &mut [u8]) -> Result<(), SnapshotError>,
) -> Result<Digest, SnapshotError> {
    let mut buffer = vec![0_u8; CHUNK];
    let mut digest = Hasher::new();
    let mut offset = 0_u64;
    while offset < size {
        let span = usize::try_from(size - offset).unwrap_or(CHUNK).min(CHUNK);
        let bytes = &mut buffer[..span];
        read(offset, bytes)?;
        let end = offset + u64::try_from(span).expect("the chunk length fits a byte offset");
        if bytes.iter().any(|&byte| byte != 0) {
            write_at(artifact, file, bytes, offset)?;
        } else if end == size {
            // The broker supplied an empty file. A write at its last byte preserves a
            // trailing hole and the exact length without another jailed syscall.
            write_at(artifact, file, &[0], end - 1)?;
        }
        digest.update(bytes);
        offset = end;
    }
    Ok(digest.finish())
}

fn write_at(
    artifact: Artifact,
    file: &File,
    bytes: &[u8],
    offset: u64,
) -> Result<(), SnapshotError> {
    file.write_all_at(bytes, offset)
        .map_err(|error| SnapshotError::io(artifact, "write checkpoint staging file", &error))
}

fn seal(artifact: Artifact, file: &File, expected: Digest) -> Result<(), SnapshotError> {
    file.sync_all()
        .map_err(|error| SnapshotError::io(artifact, "sync checkpoint staging file", &error))?;
    let length = file
        .metadata()
        .map_err(|error| SnapshotError::io(artifact, "size checkpoint staging file", &error))?
        .len();
    let mut buffer = vec![0_u8; CHUNK];
    let mut digest = Hasher::new();
    let mut offset = 0_u64;
    while offset < length {
        let span = usize::try_from(length - offset).unwrap_or(CHUNK).min(CHUNK);
        let bytes = &mut buffer[..span];
        file.read_exact_at(bytes, offset).map_err(|error| {
            SnapshotError::io(artifact, "rehash checkpoint staging file", &error)
        })?;
        digest.update(bytes);
        offset += u64::try_from(span).expect("the chunk length fits a byte offset");
    }
    if digest.finish() != expected {
        return Err(SnapshotError::StagingDigestMismatch(artifact));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounted_volume_capture_flushes_and_seals_its_own_bytes() {
        use crate::virtio::{BlockRole, FileBackend, TransferShape};

        let device_file = tempfile::tempfile().unwrap();
        let mut logical = vec![0; 2 * CHUNK];
        logical[CHUNK + 9..CHUNK + 15].copy_from_slice(b"volume");
        device_file.write_all_at(&logical, 0).unwrap();

        let mut volume = BlockDevice::new(
            BlockRole::DurableVolume,
            Box::new(FileBackend::new(device_file, false).unwrap()),
            512,
            [0; crate::virtio::BLOCK_SERIAL_LEN],
            TransferShape::Declared,
        )
        .unwrap()
        .into_serving();
        let target = tempfile::tempfile().unwrap();

        let digest = capture_block(Artifact::Volume, Some(&mut volume), Some(&target)).unwrap();

        assert_eq!(digest, Digest::of(&logical));
        let mut recovered = vec![0; logical.len()];
        target.read_exact_at(&mut recovered, 0).unwrap();
        assert_eq!(recovered, logical);
        assert!(target.metadata().unwrap().blocks() * 512 < logical.len() as u64);
        seal(Artifact::Volume, &target, digest).unwrap();
    }

    #[test]
    fn volume_capture_refuses_a_target_shared_with_another_artifact() {
        let state = tempfile::tempfile().unwrap();
        let memory = tempfile::tempfile().unwrap();
        let alias = memory.try_clone().unwrap();
        let error = validate_targets(&CheckpointTargets {
            state: &state,
            memory: &memory,
            overlay: None,
            volume: Some(&alias),
        })
        .unwrap_err();
        assert!(matches!(error, SnapshotError::NotQuiescent(_)));
    }

    #[test]
    fn checkpoint_copy_preserves_zero_spans_and_the_exact_trailing_length() {
        let mut logical = vec![0; 3 * CHUNK + 7];
        logical[CHUNK + 9] = 0x41;
        let file = tempfile::tempfile().unwrap();
        let length = u64::try_from(logical.len()).unwrap();
        let digest = copy_image(Artifact::Overlay, &file, length, |offset, span| {
            let offset = usize::try_from(offset).unwrap();
            span.copy_from_slice(&logical[offset..offset + span.len()]);
            Ok(())
        })
        .unwrap();

        let metadata = file.metadata().unwrap();
        assert_eq!(metadata.len(), length);
        assert!(metadata.blocks() * 512 < length / 2);
        assert_eq!(digest, Digest::of(&logical));
        let mut recovered = vec![0; logical.len()];
        file.read_exact_at(&mut recovered, 0).unwrap();
        assert_eq!(recovered, logical);
        seal(Artifact::Overlay, &file, digest).unwrap();
    }
}
