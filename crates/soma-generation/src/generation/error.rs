use std::{error::Error, fmt};

use crate::ImportError;

/// The Generation compiler stage that rejected an input without exposing host or guest paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompilePhase {
    /// Request, profile, toolchain, and normalized-tree resolution.
    ResolveInputs,
    /// Hostile canonical tree-manifest decoding.
    DecodeTree,
    /// Ordered tar stream emission from verified content objects.
    StreamTree,
    /// Pinned EROFS formatter invocation and checker run.
    FormatRoot,
    /// Independent EROFS traversal against the normalized tree.
    VerifyRoot,
    /// Sterile ext4 overlay-template creation.
    BuildOverlay,
    /// Read-only overlay-template verification.
    VerifyOverlay,
    /// ELF, PVH note, and configuration checks.
    VerifyKernel,
    /// Deterministic initramfs construction.
    BuildInitramfs,
    /// Initramfs decoding and allowlist verification.
    VerifyInitramfs,
    /// Canonical `SOMAGEN` manifest encoding or decoding.
    EncodeManifest,
    /// Atomic-last content-store publication.
    Publish,
    /// Cross-artifact verification of a published Generation.
    VerifyGeneration,
}

/// A stable redacted Generation compiler failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileErrorKind {
    /// The request, profile, or schema-owned artifact is structurally invalid.
    InvalidInput,
    /// A valid input feature is outside this compiler profile.
    Unsupported,
    /// A configured resource bound was exceeded.
    LimitExceeded,
    /// Content disagrees with its verified identity or another verified claim.
    Integrity,
    /// The content store does not contain the expected immutable object.
    StoreConflict,
    /// A redacted filesystem, stream, or process operation failed.
    Io,
    /// A pinned external tool is missing, has the wrong revision, or failed.
    Toolchain,
    /// The requested phase exists in the design but has no implementation yet.
    Unimplemented,
}

/// One host-path and guest-path redacted Generation compiler error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CompileError {
    phase: CompilePhase,
    kind: CompileErrorKind,
}

impl CompileError {
    pub(crate) const fn new(phase: CompilePhase, kind: CompileErrorKind) -> Self {
        Self { phase, kind }
    }

    pub(crate) const fn from_import(phase: CompilePhase, error: ImportError) -> Self {
        use crate::ImportErrorKind as ImportKind;
        let kind = match error.kind() {
            ImportKind::InvalidInput | ImportKind::NotFound | ImportKind::Ambiguous => {
                CompileErrorKind::InvalidInput
            }
            ImportKind::Unsupported => CompileErrorKind::Unsupported,
            ImportKind::LimitExceeded => CompileErrorKind::LimitExceeded,
            ImportKind::Integrity => CompileErrorKind::Integrity,
            ImportKind::StoreConflict => CompileErrorKind::StoreConflict,
            ImportKind::Io => CompileErrorKind::Io,
        };
        Self::new(phase, kind)
    }

    /// Returns the stage that rejected compilation.
    #[must_use]
    pub const fn phase(&self) -> CompilePhase {
        self.phase
    }

    /// Returns the stable redacted failure classification.
    #[must_use]
    pub const fn kind(&self) -> CompileErrorKind {
        self.kind
    }
}

impl fmt::Debug for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompileError")
            .field("phase", &self.phase)
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Generation compilation failed during {:?}: {:?}",
            self.phase, self.kind
        )
    }
}

impl Error for CompileError {}
