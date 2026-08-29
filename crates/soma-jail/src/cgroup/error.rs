//! Typed cgroup v2 failures.

use std::{error::Error, fmt};

/// Typed cgroup failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CgroupError {
    /// The root is not a cgroup2 mount.
    NotCgroup2,
    /// The root's `cgroup.controllers` lacks this controller.
    ControllerUnavailable(&'static str),
    /// The root's `cgroup.subtree_control` does not delegate this controller to leaves.
    ControllerNotDelegated(&'static str),
    /// The leaf exists already; a leaf is never reused.
    AlreadyExists,
    Create(i32),
    Open(i32),
    Write {
        file: &'static str,
        errno: i32,
    },
    Read {
        file: &'static str,
        errno: i32,
    },
    Readback {
        file: &'static str,
        expected: String,
        found: String,
    },
    /// The child is not listed in `cgroup.procs`.
    NotMember(i32),
    Remove(i32),
}

impl fmt::Display for CgroupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCgroup2 => write!(formatter, "cgroup root is not a cgroup2 mount"),
            Self::ControllerUnavailable(name) => {
                write!(formatter, "{name} controller is unavailable")
            }
            Self::ControllerNotDelegated(name) => {
                write!(formatter, "{name} controller is not delegated")
            }
            Self::AlreadyExists => write!(formatter, "cgroup leaf already exists"),
            Self::Create(errno) => write!(formatter, "cgroup leaf mkdir failed: errno {errno}"),
            Self::Open(errno) => write!(formatter, "cgroup leaf open failed: errno {errno}"),
            Self::Write { file, errno } => {
                write!(formatter, "writing {file} failed: errno {errno}")
            }
            Self::Read { file, errno } => write!(formatter, "reading {file} failed: errno {errno}"),
            Self::Readback {
                file,
                expected,
                found,
            } => {
                write!(
                    formatter,
                    "{file} reads back {found:?}, expected {expected:?}"
                )
            }
            Self::NotMember(pid) => write!(formatter, "process {pid} is not in the leaf"),
            Self::Remove(errno) => write!(formatter, "cgroup leaf rmdir failed: errno {errno}"),
        }
    }
}

impl Error for CgroupError {}
