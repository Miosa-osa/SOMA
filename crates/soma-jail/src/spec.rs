//! `JailSpec`: everything the launcher needs to constrain one VMM process.
//!
//! The specification carries no host path, TAP name, device name, or credential.
//! Host anchors such as the cgroup2 mount and the jail-root parent directory are launcher
//! inputs on the privileged side and never reach the child.

use std::{error::Error, fmt};

use crate::{manifest::DescriptorManifest, seccomp::Phase};

/// The overflow identities that a user namespace reports for unmapped principals.
const OVERFLOW_IDS: [u32; 2] = [65_534, 65_535];

/// The ephemeral unprivileged identity the VMM runs as inside its user namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
}

/// One validated cgroup v2 leaf name: a single path component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeafName(String);

impl LeafName {
    /// Accepts one non-empty component of at most 64 ASCII letters, digits, `-`, `_`, or `.`
    /// that is neither `.` nor `..` and does not start with a dot.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::LeafName`] for anything that could escape the cgroup root.
    pub fn new(name: &str) -> Result<Self, SpecError> {
        let valid = !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(SpecError::LeafName)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `cpu.max` as quota microseconds per period microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuMax {
    pub quota_us: u64,
    pub period_us: u64,
}

/// Optional `io.max` bound for one block device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoMax {
    pub major: u32,
    pub minor: u32,
    pub read_bytes_per_second: u64,
    pub write_bytes_per_second: u64,
    pub read_iops: u64,
    pub write_iops: u64,
}

/// The cgroup v2 limits written to the leaf before the child exists.
///
/// `memory.swap.max` is always `0` and `memory.oom.group` is always `1`; they are not
/// configurable because the profile fixes them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CgroupLimits {
    pub memory_max_bytes: u64,
    pub cpu_max: CpuMax,
    pub pids_max: u32,
    pub io_max: Option<IoMax>,
}

/// Resource limits applied in the child before descriptors are sealed.
///
/// `RLIMIT_CORE` is always zero and is not configurable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rlimits {
    pub nofile: u32,
    pub nproc: u32,
    pub fsize_bytes: u64,
    pub address_space_bytes: Option<u64>,
}

/// The complete launcher input for one jailed VMM process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JailSpec {
    pub identity: Identity,
    pub leaf: LeafName,
    pub limits: CgroupLimits,
    pub rlimits: Rlimits,
    pub manifest: DescriptorManifest,
    /// The seccomp phase the launcher installs before `execveat`; only `Startup` is launchable.
    pub phase: Phase,
}

/// Typed specification rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecError {
    /// The uid or gid is zero or an overflow identity.
    Identity,
    LeafName,
    MemoryMaxZero,
    CpuMaxInvalid,
    PidsMaxZero,
    IoMaxInvalid,
    /// `nofile` must cover the standard streams, every manifest slot, and the executable slot.
    NofileTooSmall {
        required: u32,
    },
    NprocZero,
    FsizeZero,
    /// Only the startup filter can `execveat` the VMM.
    UnlaunchablePhase,
}

impl fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity => write!(formatter, "uid and gid must be nonzero non-overflow values"),
            Self::LeafName => write!(formatter, "cgroup leaf name is not a single safe component"),
            Self::MemoryMaxZero => write!(formatter, "memory.max must be nonzero"),
            Self::CpuMaxInvalid => write!(
                formatter,
                "cpu.max quota and period must be nonzero and quota must not exceed period"
            ),
            Self::PidsMaxZero => write!(formatter, "pids.max must be nonzero"),
            Self::IoMaxInvalid => write!(formatter, "io.max must bound at least one dimension"),
            Self::NofileTooSmall { required } => {
                write!(formatter, "RLIMIT_NOFILE must be at least {required}")
            }
            Self::NprocZero => write!(formatter, "RLIMIT_NPROC must be nonzero"),
            Self::FsizeZero => write!(formatter, "RLIMIT_FSIZE must be nonzero"),
            Self::UnlaunchablePhase => {
                write!(formatter, "only the startup seccomp phase can launch a VMM")
            }
        }
    }
}

impl Error for SpecError {}

impl JailSpec {
    /// Checks every field so the launcher never starts construction with an unsafe value.
    ///
    /// # Errors
    ///
    /// Returns the first [`SpecError`] in declaration order.
    pub fn validate(&self) -> Result<(), SpecError> {
        let Identity { uid, gid } = self.identity;
        if uid == 0 || gid == 0 || OVERFLOW_IDS.contains(&uid) || OVERFLOW_IDS.contains(&gid) {
            return Err(SpecError::Identity);
        }
        if self.limits.memory_max_bytes == 0 {
            return Err(SpecError::MemoryMaxZero);
        }
        let CpuMax {
            quota_us,
            period_us,
        } = self.limits.cpu_max;
        if quota_us == 0 || period_us == 0 || quota_us > period_us {
            return Err(SpecError::CpuMaxInvalid);
        }
        if self.limits.pids_max == 0 {
            return Err(SpecError::PidsMaxZero);
        }
        if let Some(io) = self.limits.io_max {
            let bounded = [
                io.read_bytes_per_second,
                io.write_bytes_per_second,
                io.read_iops,
                io.write_iops,
            ]
            .iter()
            .any(|value| *value != 0);
            if !bounded {
                return Err(SpecError::IoMaxInvalid);
            }
        }
        let required = self.manifest.executable_slot() + 1;
        if self.rlimits.nofile < required {
            return Err(SpecError::NofileTooSmall { required });
        }
        if self.rlimits.nproc == 0 {
            return Err(SpecError::NprocZero);
        }
        if self.rlimits.fsize_bytes == 0 {
            return Err(SpecError::FsizeZero);
        }
        if self.phase != Phase::Startup {
            return Err(SpecError::UnlaunchablePhase);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
