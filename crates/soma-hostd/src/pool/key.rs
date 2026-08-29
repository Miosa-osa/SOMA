//! The exact pool key: host profile, Generation, CPU and memory class, overlay class, and
//! network profile.
//!
//! Two requests share a pool only when every component is byte-identical; nothing is
//! rounded or widened so a sterile worker can never be prepared for a different contract.

use sha2::{Digest, Sha256};
use soma_netd::ProfileDigest;
use soma_storage::{ClassName, OverlayClass, TemplateDigest};

use crate::{GenerationId, HostProfileDigest, MemoryClass, WorkloadClass};

/// The CPU dimension of a key: vCPU count and the certified workload class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CpuClass {
    /// Virtual processors.
    pub vcpus: u32,
    /// Workload class the overcommit policy was certified for.
    pub workload: WorkloadClass,
}

/// The memory dimension of a key: guest bytes and the explicit admission class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemoryShape {
    /// Guest memory in bytes.
    pub guest_bytes: u64,
    /// The admission class.
    pub class: MemoryClass,
}

/// The exact overlay class a sterile head is cloned from.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OverlayIdentity {
    /// Class name.
    pub name: ClassName,
    /// Class version.
    pub version: u32,
    /// Logical head size.
    pub logical_bytes: u64,
    /// Digest of the sterile template bytes.
    pub template: TemplateDigest,
}

impl OverlayIdentity {
    /// Takes the identity of one published class.
    #[must_use]
    pub fn of(class: &OverlayClass) -> Self {
        Self {
            name: class.recipe().name.clone(),
            version: class.recipe().version,
            logical_bytes: class.logical_bytes().get(),
            template: class.template_digest(),
        }
    }
}

/// The complete pool key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PoolKey {
    /// Digest of the certified host profile the workers were prepared for.
    pub host_profile: HostProfileDigest,
    /// The immutable Generation.
    pub generation: GenerationId,
    /// CPU class.
    pub cpu: CpuClass,
    /// Memory shape and class.
    pub memory: MemoryShape,
    /// Overlay class.
    pub overlay: OverlayIdentity,
    /// Digest of the network profile the sterile bundles were prepared under.
    pub network: ProfileDigest,
}

/// The 32-byte digest of one key, used for ledger records and protocol frames.
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct PoolKeyDigest([u8; 32]);

impl PoolKeyDigest {
    /// Wraps stored digest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for PoolKeyDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "pool-key({:02x}{:02x}{:02x}{:02x}..)",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

impl PoolKey {
    /// Encodes the key canonically.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let name = self.overlay.name.as_str().as_bytes();
        let mut out = Vec::with_capacity(160 + name.len());
        out.extend_from_slice(b"SOMAPOOL");
        out.extend_from_slice(self.host_profile.as_bytes());
        out.extend_from_slice(self.generation.as_bytes());
        out.extend_from_slice(&self.cpu.vcpus.to_be_bytes());
        out.push(self.cpu.workload.code());
        out.extend_from_slice(&self.memory.guest_bytes.to_be_bytes());
        out.push(self.memory.class.code());
        let expected = match self.memory.class {
            MemoryClass::Guaranteed => 0,
            MemoryClass::Elastic {
                expected_resident_bytes,
            } => expected_resident_bytes,
        };
        out.extend_from_slice(&expected.to_be_bytes());
        out.extend_from_slice(&self.overlay.version.to_be_bytes());
        out.extend_from_slice(&self.overlay.logical_bytes.to_be_bytes());
        out.extend_from_slice(self.overlay.template.as_bytes());
        out.extend_from_slice(&self.network.0);
        out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(u16::MAX).to_be_bytes());
        out.extend_from_slice(name);
        out
    }

    /// Digests the canonical encoding.
    #[must_use]
    pub fn digest(&self) -> PoolKeyDigest {
        PoolKeyDigest(Sha256::digest(self.encode()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vcpus: u32) -> PoolKey {
        PoolKey {
            host_profile: HostProfileDigest::new([1; 32]).expect("nonzero"),
            generation: GenerationId::new([2; 32]).expect("nonzero"),
            cpu: CpuClass {
                vcpus,
                workload: WorkloadClass::ApiWaiting,
            },
            memory: MemoryShape {
                guest_bytes: 512 << 20,
                class: MemoryClass::Guaranteed,
            },
            overlay: OverlayIdentity {
                name: ClassName::new("small").expect("name"),
                version: 1,
                logical_bytes: 4 << 30,
                template: TemplateDigest::from_bytes([3; 32]),
            },
            network: ProfileDigest([4; 32]),
        }
    }

    #[test]
    fn digest_is_stable_and_changes_with_every_component() {
        assert_eq!(key(1).digest(), key(1).digest());
        assert_ne!(key(1).digest(), key(2).digest());
        let mut elastic = key(1);
        elastic.memory.class = MemoryClass::Elastic {
            expected_resident_bytes: 1,
        };
        assert_ne!(elastic.digest(), key(1).digest());
        let mut renamed = key(1);
        renamed.overlay.name = ClassName::new("smal").expect("name");
        assert_ne!(renamed.digest(), key(1).digest());
        assert!(format!("{:?}", key(1).digest()).starts_with("pool-key("));
    }
}
