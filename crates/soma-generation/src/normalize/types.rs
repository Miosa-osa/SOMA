use std::{fmt, path::Path};

use soma::{OciDigest, WorkloadIdentity};

use crate::ImportedOci;

const GIB: u64 = 1024 * 1024 * 1024;

/// Explicit resource bounds for one normalized root filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootfsLimits {
    /// Maximum compressed bytes in one stored layer.
    pub max_blob_bytes: u64,
    /// Maximum aggregate expanded bytes across selected layers.
    pub max_expanded_bytes: u64,
    /// Maximum raw archive headers and final logical entries, aggregated independently.
    pub max_entries: u32,
    /// Maximum bytes in one normalized path or link target.
    pub max_path_bytes: u32,
    /// Maximum path/link metadata and extension-body bytes, aggregated independently.
    pub max_metadata_bytes: u64,
    /// Maximum bytes in one regular file.
    pub max_file_bytes: u64,
    /// Maximum aggregate regular-file bytes processed across all layers.
    pub max_content_bytes: u64,
    /// Maximum bytes in an imported or normalized completion manifest.
    pub max_manifest_bytes: u64,
}

impl Default for RootfsLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 8 * GIB,
            max_expanded_bytes: 128 * GIB,
            max_entries: 1_000_000,
            max_path_bytes: 4_096,
            max_metadata_bytes: 64 * 1024 * 1024,
            max_file_bytes: 8 * GIB,
            max_content_bytes: 128 * GIB,
            max_manifest_bytes: 512 * 1024 * 1024,
        }
    }
}

/// One explicit normalization request for a verified import in an existing store.
#[derive(Clone, Copy)]
pub struct NormalizeOciRootfs<'a> {
    pub(super) imported: &'a ImportedOci,
    pub(super) store: &'a Path,
    pub(super) limits: RootfsLimits,
}

impl<'a> NormalizeOciRootfs<'a> {
    /// Creates a request using one verified import and its existing content store.
    #[must_use]
    pub const fn new(imported: &'a ImportedOci, store: &'a Path, limits: RootfsLimits) -> Self {
        Self {
            imported,
            store,
            limits,
        }
    }
}

impl fmt::Debug for NormalizeOciRootfs<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizeOciRootfs")
            .field("imported", &self.imported)
            .field("store", &"[REDACTED]")
            .field("limits", &self.limits)
            .finish()
    }
}

/// A deterministic normalized tree and its immutable CAS completion artifact.
pub struct NormalizedRootfs {
    pub(super) workload: WorkloadIdentity,
    pub(super) source_import_manifest_digest: OciDigest,
    pub(super) tree_manifest_digest: OciDigest,
    pub(super) tree_manifest_size: u64,
    pub(super) entry_count: u32,
    pub(super) logical_file_bytes: u64,
    pub(super) content_blob_count: u32,
    pub(super) content_blob_bytes: u64,
}

impl NormalizedRootfs {
    /// Returns the imported OCI workload without a Generation identity.
    #[must_use]
    pub const fn workload(&self) -> &WorkloadIdentity {
        &self.workload
    }

    /// Returns the verified import completion digest retained as provenance.
    #[must_use]
    pub const fn source_import_manifest_digest(&self) -> &OciDigest {
        &self.source_import_manifest_digest
    }

    /// Returns the canonical normalized-tree manifest digest.
    #[must_use]
    pub const fn tree_manifest_digest(&self) -> &OciDigest {
        &self.tree_manifest_digest
    }

    /// Returns the encoded normalized-tree manifest byte length.
    #[must_use]
    pub const fn tree_manifest_size(&self) -> u64 {
        self.tree_manifest_size
    }

    /// Returns the number of logical tree entries, including the root directory.
    #[must_use]
    pub const fn entry_count(&self) -> u32 {
        self.entry_count
    }

    /// Returns the sum of final logical regular-file sizes, including hard-link aliases once.
    #[must_use]
    pub const fn logical_file_bytes(&self) -> u64 {
        self.logical_file_bytes
    }

    /// Returns the number of distinct final regular-file content objects.
    #[must_use]
    pub const fn content_blob_count(&self) -> u32 {
        self.content_blob_count
    }

    /// Returns the aggregate bytes of distinct final regular-file content objects.
    #[must_use]
    pub const fn content_blob_bytes(&self) -> u64 {
        self.content_blob_bytes
    }
}

impl fmt::Debug for NormalizedRootfs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizedRootfs")
            .field("workload", &self.workload)
            .field(
                "source_import_manifest_digest",
                &self.source_import_manifest_digest,
            )
            .field("tree_manifest_digest", &self.tree_manifest_digest)
            .field("tree_manifest_size", &self.tree_manifest_size)
            .field("entry_count", &self.entry_count)
            .field("logical_file_bytes", &self.logical_file_bytes)
            .field("content_blob_count", &self.content_blob_count)
            .field("content_blob_bytes", &self.content_blob_bytes)
            .finish()
    }
}
