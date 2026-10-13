//! Binding the volume a start names to the file the host has for its Instance.
//!
//! It lives apart from the route handlers so the handler file stays inside the source budget; the
//! decision it makes is one rule, stated once, and it is the rule that keeps a launch from
//! writing a disk the caller did not name.

use crate::envelope::ApiError;
use crate::facade::{SandboxFacade, VolumeFailure};
use crate::report::VolumeReady;
use crate::volumes::{FsckReport, VOLUME_DEVICE, VolumeBody, VolumeOperation};

/// The volume attachment one start binds, from the volume the host has for this Instance.
///
/// # Errors
///
/// Returns the host's typed refusal, or a conflict when the caller named a volume the host does
/// not have attached to this Instance.
pub(super) fn binding<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &soma::InstanceId,
    named: &str,
) -> Result<(soma::VolumeAttachment, FsckReport), ApiError> {
    let binding = facade
        .volume_binding(instance.as_str())
        .map_err(VolumeFailure::refusal)?;
    if !binding.volume.eq_ignore_ascii_case(named) {
        return Err(ApiError::new(
            409,
            "volume_instance_mismatch",
            "the volume named is not the one attached to this Instance",
            false,
        ));
    }
    let id = soma_hostd::VolumeId::from_hex(&binding.volume)
        .map_err(|_| ApiError::invalid("the host reported a volume identity it cannot carry"))?;
    // The repair this start ran travels back with the volume, so the control plane can record
    // when its data was repaired and why rather than only that the sandbox is up.
    Ok((
        soma::VolumeAttachment::new(*id.as_bytes(), binding.path),
        binding.fsck,
    ))
}

/// Detaches the volume one stop was running with, and reports it in the stop's answer.
///
/// A stop is a clean detach followed by destroying the machine, so the machine stops first and
/// the volume is released after it: a detach that fails leaves the sandbox stopped with its disk
/// intact, which is the state a retry can finish from.
///
/// # Errors
///
/// Returns the host's typed refusal, and anything that is not "this Instance has no volume".
pub(super) fn detach_for_stop<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &soma::InstanceId,
    operation: &soma::OperationId,
) -> Result<Option<VolumeReady>, ApiError> {
    let Some(owned) = facade
        .volume_owned(instance.as_str())
        .map_err(VolumeFailure::refusal)?
    else {
        return Ok(None);
    };
    // The detach is keyed by the stop's own operation, so a replayed stop replays the same detach
    // rather than asking the store for a second one under a fresh identity. The fingerprint is
    // derived from the same two values for the same reason: the store refuses a changed intent.
    let detach = VolumeBody {
        operation_id: derived("operation", operation.as_str()),
        instance_id: instance.as_str().to_owned(),
        volume_id: owned.volume.clone(),
        request_fingerprint: derived("fingerprint", operation.as_str()),
        size_class_gib: 0,
        generation_id: None,
        tenant: None,
    };
    facade
        .volume(VolumeOperation::Detach, &detach)
        .map_err(VolumeFailure::refusal)?;
    Ok(Some(VolumeReady {
        volume_id: owned.volume.to_ascii_lowercase(),
        state: "detached",
        device: VOLUME_DEVICE,
    }))
}

/// One identity a stop's detach is derived under: 32 hexadecimal characters for an operation and
/// 64 for a fingerprint, both stable across a replay of the same stop.
fn derived(label: &str, seed: &str) -> String {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(b"soma.volume.detach:");
    hasher.update(label.as_bytes());
    hasher.update(b":");
    hasher.update(seed.as_bytes());
    let digest = hasher.finalize();
    let take = if label == "fingerprint" { 32 } else { 16 };
    let mut hex = String::with_capacity(take * 2);
    for byte in &digest[..take] {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
