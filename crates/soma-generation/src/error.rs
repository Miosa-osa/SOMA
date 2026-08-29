use std::{error::Error, fmt};

/// The importer stage that rejected the input without exposing a source path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportPhase {
    /// Capability-root and OCI image-layout validation.
    OpenLayout,
    /// Bounded index traversal and unique manifest selection.
    SelectManifest,
    /// Selected OCI manifest verification.
    VerifyManifest,
    /// Selected OCI config and platform verification.
    VerifyConfig,
    /// Compressed and expanded layer verification.
    VerifyLayer,
    /// Immutable content-store publication.
    Publish,
}

/// A redacted import failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportErrorKind {
    /// The request or wire value is not structurally valid.
    InvalidInput,
    /// No image satisfies the selection.
    NotFound,
    /// More than one image satisfies the selection.
    Ambiguous,
    /// A valid OCI feature is outside this importer slice.
    Unsupported,
    /// A configured resource bound was exceeded.
    LimitExceeded,
    /// Content disagrees with a descriptor or another verified claim.
    Integrity,
    /// An existing content-addressed object is not the expected immutable value.
    StoreConflict,
    /// A redacted filesystem or stream operation failed.
    Io,
}

/// One path-redacted OCI import error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ImportError {
    phase: ImportPhase,
    kind: ImportErrorKind,
}

impl ImportError {
    pub(crate) const fn new(phase: ImportPhase, kind: ImportErrorKind) -> Self {
        Self { phase, kind }
    }

    /// Returns the stage that rejected the import.
    #[must_use]
    pub const fn phase(&self) -> ImportPhase {
        self.phase
    }

    /// Returns the stable redacted failure classification.
    #[must_use]
    pub const fn kind(&self) -> ImportErrorKind {
        self.kind
    }
}

impl fmt::Debug for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportError")
            .field("phase", &self.phase)
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "OCI import failed during {:?}: {:?}",
            self.phase, self.kind
        )
    }
}

impl Error for ImportError {}
