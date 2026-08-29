//! Evidence the launcher retains about one jailed process.

use std::fmt;

use crate::{seccomp::Phase, spec::Identity};

/// `SIGSYS`, the signal seccomp reports for a killed process.
pub const SIGSYS: i32 = 31;

/// Inode numbers of the six namespaces a process belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespaceIds {
    pub user: u64,
    pub mnt: u64,
    pub pid: u64,
    pub net: u64,
    pub ipc: u64,
    pub uts: u64,
}

impl NamespaceIds {
    /// True only when every one of the six namespaces differs.
    #[must_use]
    pub fn differs_entirely_from(&self, other: &Self) -> bool {
        self.user != other.user
            && self.mnt != other.mnt
            && self.pid != other.pid
            && self.net != other.net
            && self.ipc != other.ipc
            && self.uts != other.uts
    }
}

/// Why the jailed process stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitReason {
    Exited(i32),
    Signaled { signal: i32, core_dumped: bool },
}

impl ExitReason {
    /// A `SIGSYS` termination is the seccomp kill-process action.
    #[must_use]
    pub fn is_seccomp_kill(self) -> bool {
        matches!(self, Self::Signaled { signal: SIGSYS, .. })
    }
}

impl fmt::Display for ExitReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited(code) => write!(formatter, "exited with status {code}"),
            Self::Signaled {
                signal,
                core_dumped,
            } => {
                write!(
                    formatter,
                    "killed by signal {signal} (core dumped: {core_dumped})"
                )
            }
        }
    }
}

/// Credential and filter state read from `/proc/<pid>/status` after `execveat`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessStatus {
    pub capabilities_effective: u64,
    pub capabilities_permitted: u64,
    pub capabilities_bounding: u64,
    pub no_new_privs: bool,
    /// `Seccomp:` mode; `2` is filter mode.
    pub seccomp_mode: u32,
    pub uid: u32,
    pub gid: u32,
}

/// What the launcher can prove about one jail from the parent side.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JailEvidence {
    pub identity: Identity,
    pub namespaces: NamespaceIds,
    pub status: ProcessStatus,
    /// The leaf name relative to the cgroup root; never a host path.
    pub leaf: String,
    /// Standard streams plus manifest roles; the executable slot is closed by `execveat`.
    pub descriptor_count: u32,
    pub filter_phase: Phase,
    /// Interfaces observed in the network namespace, `lo` included.
    pub interface_count: usize,
    pub exit: Option<ExitReason>,
    /// `memory.events` `oom_kill` observed when the process was reaped.
    pub oom_kills: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_difference_requires_all_six() {
        let a = NamespaceIds {
            user: 1,
            mnt: 2,
            pid: 3,
            net: 4,
            ipc: 5,
            uts: 6,
        };
        let b = NamespaceIds {
            user: 7,
            mnt: 8,
            pid: 9,
            net: 10,
            ipc: 11,
            uts: 12,
        };
        assert!(a.differs_entirely_from(&b));
        let shared_net = NamespaceIds { net: 4, ..b };
        assert!(!a.differs_entirely_from(&shared_net));
    }

    #[test]
    fn only_sigsys_is_a_seccomp_kill() {
        assert!(
            ExitReason::Signaled {
                signal: SIGSYS,
                core_dumped: false
            }
            .is_seccomp_kill()
        );
        assert!(
            !ExitReason::Signaled {
                signal: 9,
                core_dumped: false
            }
            .is_seccomp_kill()
        );
        assert!(!ExitReason::Exited(0).is_seccomp_kill());
    }
}
