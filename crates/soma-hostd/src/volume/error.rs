//! Why a volume operation was refused, in the vocabulary the wire carries.
//!
//! Every refusal names one concrete reason so the control plane maps a typed code rather than
//! reading a message, which is the same rule the worker and Instance refusals already follow.

use std::fmt;

/// Why a volume operation was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VolumeError {
    /// No sidecar and no file name this volume.
    Missing(&'static str),
    /// The volume is attached to another Instance, or a delete is in progress.
    Busy(&'static str),
    /// The offline repair could not make the file consistent.
    FsckFailed(&'static str),
    /// The filesystem carries damage the automatic repair policy refuses to fix.
    RepairRefused(&'static str),
    /// The data key could not be unsealed, or the host has no encrypted data mount.
    Encryption(&'static str),
    /// The requested class is below the volume's own, or its geometry disagrees with its sidecar.
    Geometry(&'static str),
    /// A replay presented a changed intent under an operation that already ran.
    ChangedIntent,
    /// The durable records refused the append, or hold a record that fails its checksum.
    Ledger(&'static str),
}

impl VolumeError {
    /// The detail this refusal carries.
    #[must_use]
    pub const fn detail(&self) -> &'static str {
        match self {
            Self::Missing(detail)
            | Self::Busy(detail)
            | Self::FsckFailed(detail)
            | Self::RepairRefused(detail)
            | Self::Encryption(detail)
            | Self::Geometry(detail)
            | Self::Ledger(detail) => detail,
            Self::ChangedIntent => "changed intent under a replayed volume operation",
        }
    }
}

impl fmt::Display for VolumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for VolumeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_a_readable_detail() {
        let errors = [
            VolumeError::Missing("no sidecar"),
            VolumeError::Busy("attached elsewhere"),
            VolumeError::FsckFailed("unrepairable"),
            VolumeError::RepairRefused("damage beyond policy"),
            VolumeError::Encryption("key unsealed"),
            VolumeError::Geometry("below the volume class"),
            VolumeError::ChangedIntent,
            VolumeError::Ledger("checksum"),
        ];
        for error in errors {
            assert!(!error.detail().is_empty());
            assert!(!error.to_string().is_empty());
        }
    }
}
