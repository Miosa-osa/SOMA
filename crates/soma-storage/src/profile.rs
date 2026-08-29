//! Published overlay classes, exact-class admission, and the storage profile of a head
//! filesystem.
//!
//! Operators publish versioned [`OverlayClass`] values.
//! Admission resolves a requested writable size to exactly one class or rejects it; nothing is
//! rounded, grown, or formatted on the launch path.
//! A [`StorageProfile`] is the probed identity of the filesystem that receives heads and is the
//! only place that decides whether XFS reflink support is present.

mod dimensions;
mod naming;
mod storage;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use dimensions::{
    BlockSize, DimensionError, Ext4FeatureSet, InodePolicy, LogicalBytes, MAX_CLASS_NAME_BYTES,
    MAX_LOGICAL_BYTES, MIN_LOGICAL_BYTES, UuidPolicy,
};
pub use naming::{ClassName, FreeSpaceEvidence, MountOption, MountOptions, TemplateDigest};
pub use storage::{AdmissionRejection, FilesystemKind, ProfileRejection, StorageProfile};

/// Everything the template builder needs to produce one sterile ext4 template.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OverlayRecipe {
    /// Class name, unique together with `version`.
    pub name: ClassName,
    /// Class version; a new template always gets a new version.
    pub version: u32,
    /// Logical size of the template and of every head.
    pub logical_bytes: LogicalBytes,
    /// ext4 block size.
    pub block_size: BlockSize,
    /// ext4 UUID policy.
    pub uuid_policy: UuidPolicy,
    /// Pinned ext4 feature set.
    pub features: Ext4FeatureSet,
    /// Inode allocation policy.
    pub inode_policy: InodePolicy,
    /// Guest mount options recorded for the class.
    pub mount_options: MountOptions,
}

/// One published, certified overlay class.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OverlayClass {
    recipe: OverlayRecipe,
    template_digest: TemplateDigest,
    free_space: FreeSpaceEvidence,
}

impl OverlayClass {
    /// Publishes a class from its recipe, the digest of the template built from that recipe,
    /// and the free-space evidence admission must check.
    #[must_use]
    pub fn publish(
        recipe: OverlayRecipe,
        template_digest: TemplateDigest,
        free_space: FreeSpaceEvidence,
    ) -> Self {
        Self {
            recipe,
            template_digest,
            free_space,
        }
    }

    /// The recipe the template was built from.
    #[must_use]
    pub fn recipe(&self) -> &OverlayRecipe {
        &self.recipe
    }

    /// Digest of the sterile template bytes.
    #[must_use]
    pub fn template_digest(&self) -> TemplateDigest {
        self.template_digest
    }

    /// Free-space evidence required at admission.
    #[must_use]
    pub fn free_space(&self) -> FreeSpaceEvidence {
        self.free_space
    }

    /// Logical size of every head of this class.
    #[must_use]
    pub fn logical_bytes(&self) -> LogicalBytes {
        self.recipe.logical_bytes
    }
}

/// Why a requested writable size did not resolve to a class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassRejection {
    /// No published class has exactly the requested logical size.
    NoExactClass {
        /// Requested size in bytes.
        requested_bytes: u64,
    },
}

impl fmt::Display for ClassRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoExactClass { requested_bytes } => {
                write!(f, "no overlay class has exactly {requested_bytes} bytes")
            }
        }
    }
}

impl std::error::Error for ClassRejection {}

/// Why a catalog could not be published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// Two classes share a name and version.
    DuplicateIdentity(ClassName, u32),
    /// Two classes share a logical size, which would make resolution ambiguous.
    DuplicateLogicalBytes(u64),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateIdentity(name, version) => {
                write!(f, "class {} v{version} is published twice", name.as_str())
            }
            Self::DuplicateLogicalBytes(bytes) => {
                write!(f, "two classes have {bytes} logical bytes")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// The published classes an admission decision may choose from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassCatalog {
    classes: Vec<OverlayClass>,
}

impl ClassCatalog {
    /// Publishes a catalog whose classes have unique identities and unique logical sizes.
    ///
    /// # Errors
    ///
    /// Returns the first duplicate found.
    pub fn new(classes: Vec<OverlayClass>) -> Result<Self, CatalogError> {
        for (index, class) in classes.iter().enumerate() {
            for earlier in &classes[..index] {
                let same_name = earlier.recipe.name == class.recipe.name;
                if same_name && earlier.recipe.version == class.recipe.version {
                    return Err(CatalogError::DuplicateIdentity(
                        class.recipe.name.clone(),
                        class.recipe.version,
                    ));
                }
                if earlier.logical_bytes() == class.logical_bytes() {
                    return Err(CatalogError::DuplicateLogicalBytes(
                        class.logical_bytes().get(),
                    ));
                }
            }
        }
        Ok(Self { classes })
    }

    /// Resolves a requested size to the one class with exactly that logical size.
    ///
    /// # Errors
    ///
    /// Returns [`ClassRejection::NoExactClass`] when no class matches exactly.
    pub fn resolve(&self, requested_bytes: u64) -> Result<&OverlayClass, ClassRejection> {
        self.classes
            .iter()
            .find(|class| class.logical_bytes().get() == requested_bytes)
            .ok_or(ClassRejection::NoExactClass { requested_bytes })
    }

    /// Every published class.
    #[must_use]
    pub fn classes(&self) -> &[OverlayClass] {
        &self.classes
    }
}

#[cfg(test)]
mod tests;
