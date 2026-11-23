//! Which volume an Instance owns, and the refusal an unusable repair becomes.

use super::super::{VolumeError, VolumeSidecar, VolumeState};
use crate::InstanceId;
use crate::volume::repair::RepairOutcome;

/// The refusal one unusable repair produces, shared by the attach and the start paths.
pub(in crate::volume::store) fn refusal(outcome: RepairOutcome) -> VolumeError {
    match outcome {
        RepairOutcome::Refused => {
            VolumeError::RepairRefused("the volume needs a repair beyond the policy")
        }
        _ => VolumeError::FsckFailed("the offline repair could not make the volume consistent"),
    }
}

pub(in crate::volume::store) fn owned_volume<'a>(
    volumes: impl Iterator<Item = &'a VolumeSidecar>,
    instance: InstanceId,
) -> Result<&'a VolumeSidecar, VolumeError> {
    let mut volumes = volumes
        .filter(|sidecar| sidecar.instance == instance && sidecar.state != VolumeState::Deleting);
    let volume = volumes
        .next()
        .ok_or(VolumeError::Missing("no volume belongs to this Instance"))?;
    if volumes.next().is_some() {
        return Err(VolumeError::Busy(
            "multiple volumes belong to this Instance",
        ));
    }
    Ok(volume)
}
