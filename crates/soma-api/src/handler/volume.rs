//! Binding the volume a start names to the file the host has for its Instance.
//!
//! It lives apart from the route handlers so the handler file stays inside the source budget; the
//! decision it makes is one rule, stated once, and it is the rule that keeps a launch from
//! writing a disk the caller did not name.

use crate::envelope::ApiError;
use crate::facade::{SandboxFacade, VolumeFailure};
use crate::volumes::FsckReport;

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
