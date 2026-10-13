//! The typed refusals of the durable data volume surface, and the state names it reports.
//!
//! The statuses are the ones the interface fixes, and the codes are the daemon's own stable
//! vocabulary rather than a second one invented here, so a client maps on the code it was sent.

use soma_hostd::FailureCode;

use super::VolumeFailure;
use crate::capability::MissingCapability;
use crate::envelope::ApiError;

impl VolumeFailure {
    /// The refusal this failure renders as.
    #[must_use]
    pub fn refusal(self) -> ApiError {
        match self {
            Self::Unconfigured => MissingCapability::DurableVolumes.error(),
            Self::Malformed => ApiError::new(
                400,
                "invalid_volume_request",
                "the volume request is malformed",
                false,
            ),
            Self::Transport => transport(),
            Self::Refused(code) => refusal_for(code),
        }
    }
}

/// The name of one volume state code.
#[must_use]
pub const fn state_name(code: u8) -> &'static str {
    match code {
        1 => "attached",
        2 => "detached",
        _ => "deleting",
    }
}

/// The HTTP refusal for one daemon failure code.
pub(super) fn refusal_for(code: u16) -> ApiError {
    match FailureCode::from_wire(code) {
        Some(FailureCode::VolumeMissing) => not_found(),
        Some(FailureCode::VolumeBusy) => conflict("VolumeBusy", "the volume is attached elsewhere"),
        Some(FailureCode::Conflict) => {
            conflict("Conflict", "a replayed operation carried a changed intent")
        }
        Some(FailureCode::VolumeFsckFailed) => conflict(
            "VolumeFsckFailed",
            "the offline repair could not make the volume consistent",
        ),
        Some(FailureCode::VolumeRepairRefused) => conflict(
            "VolumeRepairRefused",
            "the volume needs a repair beyond the automatic policy",
        ),
        Some(FailureCode::VolumeEncryption) => conflict(
            "VolumeEncryption",
            "the volume's data key could not be unsealed",
        ),
        Some(FailureCode::VolumeGeometry) => conflict(
            "VolumeGeometry",
            "the requested class or geometry does not fit the volume",
        ),
        Some(FailureCode::Exhausted | FailureCode::Capacity) => ApiError::new(
            507,
            "volume_capacity",
            "the host has no capacity for another volume",
            true,
        ),
        Some(FailureCode::Ledger) => ApiError::new(
            500,
            "volume_ledger",
            "the durable record refused the volume operation",
            true,
        ),
        _ => ApiError::new(
            500,
            "volume_refused",
            "the host refused the volume operation",
            false,
        ),
    }
}

fn not_found() -> ApiError {
    ApiError::new(404, "VolumeMissing", "no volume has that identity", false)
}

fn transport() -> ApiError {
    ApiError::new(
        503,
        "volume_transport",
        "the host control socket could not be reached",
        true,
    )
}

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError::new(409, code, message, false)
}
