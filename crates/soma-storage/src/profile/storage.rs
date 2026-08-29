//! Probed identity of the filesystem that receives heads and the admission check against a
//! published class.

#[cfg(target_os = "linux")]
mod probe;

use std::fmt;

use serde::{Deserialize, Serialize};

use super::OverlayClass;

/// Filesystem kinds the profile can certify.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilesystemKind {
    /// XFS with the `reflink=1` feature proven by a successful `FICLONE`.
    XfsReflink,
}

/// Probed identity of the filesystem that receives heads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StorageProfile {
    filesystem: FilesystemKind,
    mount_id: u64,
    device: u64,
    block_size: u64,
    free_bytes: u64,
}

impl StorageProfile {
    /// The certified filesystem kind.
    #[must_use]
    pub fn filesystem(&self) -> FilesystemKind {
        self.filesystem
    }

    /// Kernel mount identifier of the head directory.
    #[must_use]
    pub fn mount_id(&self) -> u64 {
        self.mount_id
    }

    /// Device number of the head directory.
    #[must_use]
    pub fn device(&self) -> u64 {
        self.device
    }

    /// Filesystem block size reported by `statfs`.
    #[must_use]
    pub fn block_size(&self) -> u64 {
        self.block_size
    }

    /// Free bytes available to unprivileged writers when the profile was probed.
    #[must_use]
    pub fn free_bytes(&self) -> u64 {
        self.free_bytes
    }

    /// Admits a class only when the probed free space covers its evidence requirement.
    ///
    /// # Errors
    ///
    /// Returns the shortfall.
    pub fn admit(&self, class: &OverlayClass) -> Result<(), AdmissionRejection> {
        let required = class.free_space().minimum_free_bytes;
        if self.free_bytes < required {
            return Err(AdmissionRejection::InsufficientFreeSpace {
                required,
                available: self.free_bytes,
            });
        }
        Ok(())
    }
}

/// Why a class was not admitted on this profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionRejection {
    /// The filesystem had less free space than the class requires.
    InsufficientFreeSpace {
        /// Required free bytes.
        required: u64,
        /// Probed free bytes.
        available: u64,
    },
}

impl fmt::Display for AdmissionRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientFreeSpace {
                required,
                available,
            } => {
                write!(f, "{available} free bytes is below the required {required}")
            }
        }
    }
}

impl std::error::Error for AdmissionRejection {}

/// Why a directory cannot receive heads.
#[derive(Debug)]
pub enum ProfileRejection {
    /// The filesystem is not XFS.
    NotXfs {
        /// The `statfs` magic that was observed.
        magic: i64,
    },
    /// The kernel refused `FICLONE`, so the mount lacks `reflink=1` or the kernel lacks support.
    ReflinkUnsupported,
    /// A probe system call failed for a reason other than missing reflink support.
    Probe(std::io::Error),
}

impl fmt::Display for ProfileRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotXfs { magic } => write!(f, "filesystem magic {magic:#x} is not XFS"),
            Self::ReflinkUnsupported => f.write_str("the filesystem refused FICLONE"),
            Self::Probe(error) => write!(f, "profile probe failed: {error}"),
        }
    }
}

impl std::error::Error for ProfileRejection {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{
        BlockSize, ClassName, Ext4FeatureSet, FreeSpaceEvidence, InodePolicy, LogicalBytes,
        MountOptions, OverlayRecipe, TemplateDigest, UuidPolicy,
    };

    fn class(mib: u64, free_mib: u64) -> OverlayClass {
        let recipe = OverlayRecipe {
            name: ClassName::new("ovl").expect("name"),
            version: 1,
            logical_bytes: LogicalBytes::new(mib * 1024 * 1024, BlockSize::B4096).expect("size"),
            block_size: BlockSize::B4096,
            uuid_policy: UuidPolicy::Derived,
            features: Ext4FeatureSet::V1,
            inode_policy: InodePolicy::bytes_per_inode(16384).expect("ratio"),
            mount_options: MountOptions::new(&[]),
        };
        OverlayClass::publish(
            recipe,
            TemplateDigest::from_bytes([1; 32]),
            FreeSpaceEvidence {
                minimum_free_bytes: free_mib * 1024 * 1024,
            },
        )
    }

    #[test]
    fn admission_requires_the_free_space_evidence() {
        let profile = StorageProfile {
            filesystem: FilesystemKind::XfsReflink,
            mount_id: 7,
            device: 0x700,
            block_size: 4096,
            free_bytes: 100 * 1024 * 1024,
        };
        assert_eq!(profile.filesystem(), FilesystemKind::XfsReflink);
        assert_eq!(profile.mount_id(), 7);
        assert_eq!(profile.device(), 0x700);
        assert_eq!(profile.block_size(), 4096);
        assert_eq!(profile.free_bytes(), 100 * 1024 * 1024);
        assert_eq!(profile.admit(&class(1024, 64)), Ok(()));
        assert_eq!(
            profile.admit(&class(4096, 256)),
            Err(AdmissionRejection::InsufficientFreeSpace {
                required: 256 * 1024 * 1024,
                available: 100 * 1024 * 1024,
            })
        );
    }

    #[test]
    fn rejections_render_their_reason() {
        assert_eq!(
            ProfileRejection::NotXfs { magic: 0xef53 }.to_string(),
            "filesystem magic 0xef53 is not XFS"
        );
        assert_eq!(
            ProfileRejection::ReflinkUnsupported.to_string(),
            "the filesystem refused FICLONE"
        );
        assert_eq!(
            AdmissionRejection::InsufficientFreeSpace {
                required: 2,
                available: 1
            }
            .to_string(),
            "1 free bytes is below the required 2"
        );
    }
}
