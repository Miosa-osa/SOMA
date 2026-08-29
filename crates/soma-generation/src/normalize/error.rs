use std::{error::Error, fmt};

/// The normalization stage that rejected an input without exposing guest or host paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizePhase {
    /// Request, store, and imported completion validation.
    OpenImport,
    /// Stored layer and expanded-stream verification.
    VerifyLayer,
    /// OCI changeset application to the logical tree.
    ApplyLayer,
    /// Canonical rootfs manifest encoding.
    EncodeManifest,
    /// Immutable content or completion publication.
    Publish,
}

/// A stable redacted rootfs normalization failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizeErrorKind {
    /// The request or schema-owned artifact is structurally invalid.
    InvalidInput,
    /// A valid filesystem feature is outside this normalization profile.
    Unsupported,
    /// A configured resource bound was exceeded.
    LimitExceeded,
    /// Stored or expanded content disagrees with its verified identity.
    Integrity,
    /// The content store does not contain the expected immutable object.
    StoreConflict,
    /// A redacted filesystem or stream operation failed.
    Io,
}

/// One host-path and guest-path redacted normalization error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NormalizeError {
    phase: NormalizePhase,
    kind: NormalizeErrorKind,
}

impl NormalizeError {
    pub(crate) const fn new(phase: NormalizePhase, kind: NormalizeErrorKind) -> Self {
        Self { phase, kind }
    }

    pub(super) const fn from_import(phase: NormalizePhase, error: crate::ImportError) -> Self {
        use crate::ImportErrorKind as ImportKind;
        let kind = match error.kind() {
            ImportKind::InvalidInput | ImportKind::NotFound | ImportKind::Ambiguous => {
                NormalizeErrorKind::InvalidInput
            }
            ImportKind::Unsupported => NormalizeErrorKind::Unsupported,
            ImportKind::LimitExceeded => NormalizeErrorKind::LimitExceeded,
            ImportKind::Integrity => NormalizeErrorKind::Integrity,
            ImportKind::StoreConflict => NormalizeErrorKind::StoreConflict,
            ImportKind::Io => NormalizeErrorKind::Io,
        };
        Self::new(phase, kind)
    }

    /// Returns the stage that rejected normalization.
    #[must_use]
    pub const fn phase(&self) -> NormalizePhase {
        self.phase
    }

    /// Returns the stable redacted failure classification.
    #[must_use]
    pub const fn kind(&self) -> NormalizeErrorKind {
        self.kind
    }
}

impl fmt::Debug for NormalizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizeError")
            .field("phase", &self.phase)
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NormalizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "rootfs normalization failed during {:?}: {:?}",
            self.phase, self.kind
        )
    }
}

impl Error for NormalizeError {}
