//! One typed failure for every way a live capture or restore can stop.
//!
//! Nothing here carries a host path, descriptor number, guest byte, or key: a failure names
//! the step and the classification, so a rejected snapshot is diagnosable without becoming a
//! disclosure channel.

use std::{error::Error, fmt};

use crate::snapshot::{
    WireError,
    capture::CaptureOrderError,
    compatibility::Incompatibility,
    device_state::DeviceStateError,
    kvm_state::KvmStateError,
    manifest::ManifestError,
    memory::{MappingError, MemoryError},
    readiness::ReadinessRefusal,
    restore::RestoreOrderError,
    section::SectionError,
};
use crate::virtio::{Slot, SlotRestoreError};
use crate::x86_64::error::MachineError;

/// Which artifact or step a failure belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Artifact {
    Memory,
    Overlay,
    State,
    /// The immutable Generation root, hashed only for its installation-time identity.
    Root,
    Directory,
}

/// Why a live capture or restore failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The machine layer failed; the phase names the step.
    Machine(MachineError),
    /// A quiesce precondition could not be proven at the capture point.
    NotQuiescent(&'static str),
    /// The capture steps were driven out of the fixed order.
    CaptureOrder(CaptureOrderError),
    /// The restore steps were driven out of the fixed order.
    RestoreOrder(RestoreOrderError),
    /// The authenticated readiness evidence was absent, spent, or did not authenticate.
    Readiness(ReadinessRefusal),
    /// A KVM ioctl failed while reading or installing state.
    Ioctl { operation: &'static str, errno: i32 },
    /// A KVM table was written only in part, so the state is neither old nor new.
    PartialTable {
        operation: &'static str,
        expected: usize,
        actual: usize,
    },
    /// The host cannot be trusted to hold this vCPU's extended state in 4,096 bytes.
    XsaveTooLarge(i32),
    /// The vCPU carries nested-virtualization state, which version 1 refuses to certify.
    NestedStatePresent,
    /// A device negotiated something other than this implementation's feature allowlist.
    FeatureNegotiation { slot: Slot, negotiated: u64 },
    /// A KVM state group could not be converted or encoded.
    KvmState(KvmStateError),
    /// A device state could not be converted or encoded.
    DeviceState(DeviceStateError),
    /// The canonical device state does not reproduce the live device state.
    DeviceStateNotCanonical(Slot),
    /// A restored slot was rejected.
    SlotRestore(SlotRestoreError),
    /// The manifest could not be built or decoded.
    Manifest(ManifestError),
    /// A section could not be built or decoded.
    Section(SectionError),
    /// A section payload could not be decoded.
    Wire(WireError),
    /// The memory descriptor rejected the object.
    Memory(MemoryError),
    /// The memory object could not be mapped privately.
    Mapping(MappingError),
    /// The host does not satisfy the snapshot's requirements.
    Incompatible(Incompatibility),
    /// A required section is absent from the manifest.
    MissingSection(&'static str),
    /// The repair-point marker is absent or does not describe a pre-launch capture.
    RepairPointMarker,
    /// An artifact file operation failed.
    Io {
        artifact: Artifact,
        operation: &'static str,
        errno: i32,
    },
    /// An artifact is already published, so the capture would overwrite a certified object.
    AlreadyPublished(Artifact),
    /// A staging object read back differently from the bytes written to it.
    StagingDigestMismatch(Artifact),
    /// The manifest decoded from the staged bytes is not the manifest that was built.
    StagingNotCanonical,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Machine(error) => write!(formatter, "{error}"),
            Self::NotQuiescent(reason) => write!(formatter, "not quiescent: {reason}"),
            Self::CaptureOrder(error) => write!(formatter, "{error}"),
            Self::RestoreOrder(error) => write!(formatter, "{error}"),
            Self::Readiness(refusal) => write!(formatter, "{refusal}"),
            Self::Ioctl { operation, errno } => {
                write!(formatter, "{operation} failed with errno {errno}")
            }
            Self::PartialTable {
                operation,
                expected,
                actual,
            } => write!(
                formatter,
                "{operation} applied {actual} of {expected} entries"
            ),
            Self::XsaveTooLarge(size) => write!(
                formatter,
                "the host reports a {size}-byte XSAVE area, above the certified 4096"
            ),
            Self::NestedStatePresent => {
                formatter.write_str("the vCPU carries nested state, unsupported in version 1")
            }
            Self::FeatureNegotiation { slot, negotiated } => write!(
                formatter,
                "{slot:?} negotiated {negotiated:#x}, not the implementation allowlist"
            ),
            Self::KvmState(error) => write!(formatter, "{error}"),
            Self::DeviceState(error) => write!(formatter, "{error}"),
            Self::DeviceStateNotCanonical(slot) => write!(
                formatter,
                "the canonical state of {slot:?} does not reproduce the live device"
            ),
            Self::SlotRestore(error) => write!(formatter, "{error}"),
            Self::Manifest(error) => write!(formatter, "{error}"),
            Self::Section(error) => write!(formatter, "{error}"),
            Self::Wire(error) => write!(formatter, "{error}"),
            Self::Memory(error) => write!(formatter, "{error}"),
            Self::Mapping(error) => write!(formatter, "{error}"),
            Self::Incompatible(reason) => write!(formatter, "incompatible snapshot: {reason}"),
            Self::MissingSection(role) => write!(formatter, "section {role} is absent"),
            Self::RepairPointMarker => {
                formatter.write_str("the snapshot does not carry a pre-launch repair-point marker")
            }
            Self::Io {
                artifact,
                operation,
                errno,
            } => write!(
                formatter,
                "{operation} on the {artifact:?} artifact failed with errno {errno}"
            ),
            Self::AlreadyPublished(artifact) => {
                write!(formatter, "the {artifact:?} artifact is already published")
            }
            Self::StagingDigestMismatch(artifact) => write!(
                formatter,
                "the staged {artifact:?} artifact reads back differently from what was written"
            ),
            Self::StagingNotCanonical => {
                formatter.write_str("the staged manifest does not decode to the built manifest")
            }
        }
    }
}

impl Error for SnapshotError {}

macro_rules! from_error {
    ($($source:ty => $variant:ident),* $(,)?) => {
        $(impl From<$source> for SnapshotError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        })*
    };
}

from_error! {
    MachineError => Machine,
    CaptureOrderError => CaptureOrder,
    RestoreOrderError => RestoreOrder,
    ReadinessRefusal => Readiness,
    KvmStateError => KvmState,
    DeviceStateError => DeviceState,
    SlotRestoreError => SlotRestore,
    ManifestError => Manifest,
    SectionError => Section,
    WireError => Wire,
    MemoryError => Memory,
    MappingError => Mapping,
    Incompatibility => Incompatible,
}

impl SnapshotError {
    pub(super) fn ioctl(operation: &'static str, error: kvm_ioctls::Error) -> Self {
        Self::Ioctl {
            operation,
            errno: error.errno(),
        }
    }

    pub(super) fn io(artifact: Artifact, operation: &'static str, error: &std::io::Error) -> Self {
        Self::Io {
            artifact,
            operation,
            errno: error.raw_os_error().unwrap_or(0),
        }
    }
}

impl From<crate::snapshot::kvm_state::bindings::BindingError> for SnapshotError {
    fn from(error: crate::snapshot::kvm_state::bindings::BindingError) -> Self {
        use crate::snapshot::kvm_state::bindings::BindingError;
        match error {
            BindingError::State(state) => Self::KvmState(state),
            BindingError::FlagOutOfRange { field, .. }
            | BindingError::TableTooLarge { field, .. } => Self::Ioctl {
                operation: field,
                errno: 0,
            },
            BindingError::UnsupportedRouteType(_) => Self::Ioctl {
                operation: "irq routing type",
                errno: 0,
            },
            BindingError::UnsupportedNestedFormat(_) => Self::Ioctl {
                operation: "nested state format",
                errno: 0,
            },
            BindingError::XsaveLength(_) => Self::Ioctl {
                operation: "xsave length",
                errno: 0,
            },
        }
    }
}
