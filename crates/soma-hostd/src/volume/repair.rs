//! The offline repair receipt an attach or a start produces.
//!
//! The repair runs on the host, on the detached file, under the volume lock, before any machine
//! can hold the volume, so a guest that is already running can never race a live repair.
//! The receipt is durable: it is appended to the write-ahead records and returned in the attach
//! and start answers, so a customer can be told when their data was repaired and why, in the
//! same spirit as ADR 0034's durable expiry receipts.

use std::fmt;
use std::path::Path;

use crate::VolumeId;

/// The fsck tool a Step A repair runs.
pub const REPAIR_TOOL: &str = "e2fsck";
/// The tool a Step A volume is created with.
pub const FORMAT_TOOL: &str = "mkfs.ext4";

/// Why a volume file could not be created.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateError {
    /// The path already holds data, so making a filesystem there would destroy it.
    Occupied,
    /// The requested size is not one a filesystem can be created with.
    Size(u64),
    /// The tool could not be run, or it refused the file; the value is its exit code, or -1.
    Tool(i32),
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Occupied => write!(formatter, "the volume path already holds data"),
            Self::Size(bytes) => write!(formatter, "a volume of {bytes} bytes cannot be created"),
            Self::Tool(code) => write!(formatter, "{FORMAT_TOOL} exited {code}"),
        }
    }
}

/// The offline filesystem operations a store runs on a detached volume file.
///
/// It is a seam because the real implementation invokes the two ext4 tools on a detached file,
/// which needs a real file and a real host, while a test needs a deterministic answer.
pub trait VolumeRepair {
    /// Creates the volume's filesystem, for a volume that does not have one yet.
    ///
    /// A store calls this exactly once, when it creates a volume, before any repair runs, so the
    /// repair never sees a path that does not exist. The default does nothing, which is what a
    /// policy double and a store that never creates want.
    ///
    /// # Errors
    ///
    /// Returns [`CreateError::Occupied`] rather than formatting a path that already holds data.
    fn create(&self, _volume: VolumeId, _file: &Path, _size_bytes: u64) -> Result<(), CreateError> {
        Ok(())
    }

    /// Repairs one volume file and returns the receipt, whatever the outcome.
    fn repair(&self, volume: VolumeId, file: &Path) -> RepairReceipt;
}

/// How one offline repair ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RepairOutcome {
    /// The filesystem was already consistent; nothing was changed.
    Clean = 1,
    /// The filesystem was inconsistent and the automatic policy repaired it.
    Repaired = 2,
    /// The filesystem needs a repair the automatic policy refuses to make.
    Refused = 3,
    /// The repair could not run, or the file is not a filesystem it understands.
    Failed = 4,
}

impl RepairOutcome {
    /// Decodes one outcome code, or `None` when this build does not know it.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Clean,
            2 => Self::Repaired,
            3 => Self::Refused,
            4 => Self::Failed,
            _ => return None,
        })
    }

    /// Returns the stable wire code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Whether this outcome leaves the filesystem usable.
    #[must_use]
    pub const fn is_usable(self) -> bool {
        matches!(self, Self::Clean | Self::Repaired)
    }
}

/// One durable statement that a repair ran on a volume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairReceipt {
    /// The volume this repair ran on.
    pub volume: VolumeId,
    /// How the repair ended.
    pub outcome: RepairOutcome,
    /// The `fsck` exit code, saturated into a signed byte; it is not a status the wire carries.
    pub exit_code: i8,
    /// Unix nanoseconds when the repair ran.
    pub ran_at_nanos: u64,
}

impl RepairReceipt {
    /// Records one repair outcome.
    #[must_use]
    pub const fn new(
        volume: VolumeId,
        outcome: RepairOutcome,
        exit_code: i8,
        ran_at_nanos: u64,
    ) -> Self {
        Self {
            volume,
            outcome,
            exit_code,
            ran_at_nanos,
        }
    }

    /// Whether this receipt says the volume may be mounted.
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        self.outcome.is_usable()
    }
}

impl fmt::Display for RepairReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} exit {} at {}",
            REPAIR_TOOL, self.exit_code, self.ran_at_nanos
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume() -> VolumeId {
        VolumeId::new([7; 16]).expect("nonzero")
    }

    #[test]
    fn only_clean_and_repaired_leave_the_volume_usable() {
        assert!(RepairOutcome::Clean.is_usable());
        assert!(RepairOutcome::Repaired.is_usable());
        assert!(!RepairOutcome::Refused.is_usable());
        assert!(!RepairOutcome::Failed.is_usable());
    }

    #[test]
    fn every_outcome_has_a_distinct_stable_code() {
        assert_eq!(RepairOutcome::Clean.code(), 1);
        assert_eq!(RepairOutcome::Repaired.code(), 2);
        assert_eq!(RepairOutcome::Refused.code(), 3);
        assert_eq!(RepairOutcome::Failed.code(), 4);
        assert_eq!(RepairOutcome::from_code(5), None);
    }

    #[test]
    fn a_receipt_reports_whether_the_volume_may_be_mounted() {
        let clean = RepairReceipt::new(volume(), RepairOutcome::Clean, 0, 1);
        assert!(clean.is_usable());
        let refused = RepairReceipt::new(volume(), RepairOutcome::Refused, 4, 1);
        assert!(!refused.is_usable());
        assert!(refused.to_string().contains(REPAIR_TOOL));
    }
}
