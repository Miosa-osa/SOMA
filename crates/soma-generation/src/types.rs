use std::{fmt, path::Path};

use soma::{OciDigest, OciPlatform, WorkloadIdentity};

const DEFAULT_MAX_DESCRIPTORS: u32 = 256;
const DEFAULT_MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BLOB_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const DEFAULT_MAX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024 * 1024;

/// Explicit resource bounds for one OCI import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportLimits {
    /// Maximum supported descriptors across traversed indexes and the selected manifest.
    pub max_descriptors: u32,
    /// Maximum compressed or metadata bytes in any one source or generated artifact.
    pub max_blob_bytes: u64,
    /// Maximum unique bytes referenced by the selected source and traversal path.
    pub max_total_blob_bytes: u64,
    /// Maximum aggregate expanded bytes across all selected layers.
    pub max_expanded_bytes: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_descriptors: DEFAULT_MAX_DESCRIPTORS,
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_total_blob_bytes: DEFAULT_MAX_TOTAL_BLOB_BYTES,
            max_expanded_bytes: DEFAULT_MAX_EXPANDED_BYTES,
        }
    }
}

/// Selects one OCI image by platform or by an already resolved immutable identity.
#[derive(Clone, Copy, Debug)]
pub enum OciSelection<'a> {
    /// Selects the unique image compatible with this platform.
    Platform(&'a OciPlatform),
    /// Selects this exact manifest and refines a compatible generic platform when verified.
    /// The identity's index digest is copied only as caller-supplied registry provenance.
    /// It is neither resolved nor authenticated against the local layout indexes.
    Exact(&'a WorkloadIdentity),
}

/// One explicit local OCI image-layout import request.
#[derive(Clone, Copy)]
pub struct ImportOciLayout<'a> {
    pub(crate) layout: &'a Path,
    pub(crate) store: &'a Path,
    pub(crate) selection: OciSelection<'a>,
    pub(crate) limits: ImportLimits,
}

impl fmt::Debug for ImportOciLayout<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportOciLayout")
            .field("layout", &"[REDACTED]")
            .field("store", &"[REDACTED]")
            .field("selection", &self.selection)
            .field("limits", &self.limits)
            .finish()
    }
}

impl<'a> ImportOciLayout<'a> {
    /// Creates a request for existing layout and store roots.
    #[must_use]
    pub const fn new(
        layout: &'a Path,
        store: &'a Path,
        selection: OciSelection<'a>,
        limits: ImportLimits,
    ) -> Self {
        Self {
            layout,
            store,
            selection,
            limits,
        }
    }
}

/// A verified OCI input and its immutable CAS completion artifact.
pub struct ImportedOci {
    pub(crate) workload: WorkloadIdentity,
    pub(crate) import_manifest_digest: OciDigest,
    pub(crate) import_manifest_size: u64,
    pub(crate) stored_blob_count: u32,
    pub(crate) stored_bytes: u64,
    pub(crate) traversed_indexes: Vec<OciDigest>,
}

impl ImportedOci {
    /// Returns the selected OCI workload with its effective platform and no Generation identity.
    #[must_use]
    pub const fn workload(&self) -> &WorkloadIdentity {
        &self.workload
    }

    /// Returns the digest of the deterministic SOMA import manifest.
    #[must_use]
    pub const fn import_manifest_digest(&self) -> &OciDigest {
        &self.import_manifest_digest
    }

    /// Returns the encoded byte length of the deterministic SOMA import manifest.
    #[must_use]
    pub const fn import_manifest_size(&self) -> u64 {
        self.import_manifest_size
    }

    /// Returns the number of unique CAS artifacts referenced by this import.
    #[must_use]
    pub const fn stored_blob_count(&self) -> u32 {
        self.stored_blob_count
    }

    /// Returns the aggregate bytes of unique CAS artifacts referenced by this import.
    #[must_use]
    pub const fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }

    /// Returns local traversal-index digests in top-to-selected order.
    #[must_use]
    pub fn traversed_indexes(&self) -> &[OciDigest] {
        &self.traversed_indexes
    }
}

impl fmt::Debug for ImportedOci {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportedOci")
            .field("workload", &self.workload)
            .field("import_manifest_digest", &self.import_manifest_digest)
            .field("import_manifest_size", &self.import_manifest_size)
            .field("stored_blob_count", &self.stored_blob_count)
            .field("stored_bytes", &self.stored_bytes)
            .field("traversed_index_count", &self.traversed_indexes.len())
            .finish()
    }
}
