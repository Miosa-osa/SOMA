//! The machine bounds one compiler-policy version admits, and the versions this compiler has
//! limits for.

/// The machine bounds one compiler-policy version admits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileLimits {
    /// The largest vCPU count.
    pub max_vcpus: u16,
    /// The largest guest RAM in MiB.
    pub max_memory_mib: u64,
}

impl ProfileLimits {
    /// The limits of one compiler-policy version, or `None` for a version with no limits.
    #[must_use]
    pub const fn for_version(version: u16) -> Option<Self> {
        match version {
            1 => Some(Self {
                max_vcpus: soma_kvm::V1_MAX_VCPUS,
                max_memory_mib: 3 * 1024,
            }),
            2 => Some(Self {
                max_vcpus: soma_kvm::V2_MAX_VCPUS,
                max_memory_mib: 16 * 1024,
            }),
            _ => None,
        }
    }
}
