//! Namespace construction and parent-side namespace evidence.
//!
//! The child is cloned directly into user, mount, PID, network, IPC, and UTS namespaces; the
//! parent writes the single-entry identity maps, then reads namespace inodes, capabilities,
//! and interfaces through `/proc/<pid>` so no claim depends on the child's cooperation.
//! The empty root is entered by [`root::enter_empty_root`].

mod root;

use std::{error::Error, fmt, fs, io};

pub use root::RootStep;
pub(crate) use root::enter_empty_root;

use crate::{
    evidence::{NamespaceIds, ProcessStatus},
    spec::Identity,
};

/// The namespaces every jail child receives.
pub const CLONE_NAMESPACES: u64 = (libc::CLONE_NEWUSER
    | libc::CLONE_NEWNS
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUTS) as u64;

/// Typed namespace failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceError {
    IdMap {
        file: &'static str,
        errno: i32,
    },
    Read {
        what: &'static str,
        errno: i32,
    },
    Malformed(&'static str),
    /// The child shares at least one namespace with the launcher.
    NotIsolated,
    /// More interfaces exist than `lo` plus the transferred TAP endpoints.
    UnexpectedInterfaces {
        found: usize,
        allowed: usize,
    },
}

impl fmt::Display for NamespaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdMap { file, errno } => {
                write!(formatter, "writing {file} failed: errno {errno}")
            }
            Self::Read { what, errno } => write!(formatter, "reading {what} failed: errno {errno}"),
            Self::Malformed(what) => write!(formatter, "{what} is malformed"),
            Self::NotIsolated => write!(formatter, "child shares a namespace with the launcher"),
            Self::UnexpectedInterfaces { found, allowed } => {
                write!(
                    formatter,
                    "network namespace has {found} interfaces, at most {allowed} allowed"
                )
            }
        }
    }
}

impl Error for NamespaceError {}

fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

/// Writes `setgroups`, `gid_map`, and `uid_map` for `pid` as single identity entries.
///
/// A root launcher may map any identity; an unprivileged launcher may only map its own.
///
/// # Errors
///
/// Returns [`NamespaceError::IdMap`] naming the file the kernel rejected.
pub fn write_id_maps(pid: i32, identity: Identity) -> Result<(), NamespaceError> {
    let write = |file: &'static str, value: String| {
        fs::write(format!("/proc/{pid}/{file}"), value).map_err(|error| NamespaceError::IdMap {
            file,
            errno: errno_of(&error),
        })
    };
    write("setgroups", "deny\n".to_owned())?;
    write("gid_map", format!("{0} {0} 1\n", identity.gid))?;
    write("uid_map", format!("{0} {0} 1\n", identity.uid))
}

fn namespace_inode(pid_text: &str, name: &'static str) -> Result<u64, NamespaceError> {
    let link = fs::read_link(format!("/proc/{pid_text}/ns/{name}")).map_err(|error| {
        NamespaceError::Read {
            what: name,
            errno: errno_of(&error),
        }
    })?;
    let text = link.to_str().ok_or(NamespaceError::Malformed(name))?;
    let (prefix, rest) = text
        .split_once(":[")
        .ok_or(NamespaceError::Malformed(name))?;
    if prefix != name {
        return Err(NamespaceError::Malformed(name));
    }
    rest.strip_suffix(']')
        .and_then(|inode| inode.parse().ok())
        .ok_or(NamespaceError::Malformed(name))
}

fn namespace_ids(pid_text: &str) -> Result<NamespaceIds, NamespaceError> {
    Ok(NamespaceIds {
        user: namespace_inode(pid_text, "user")?,
        mnt: namespace_inode(pid_text, "mnt")?,
        pid: namespace_inode(pid_text, "pid")?,
        net: namespace_inode(pid_text, "net")?,
        ipc: namespace_inode(pid_text, "ipc")?,
        uts: namespace_inode(pid_text, "uts")?,
    })
}

/// The namespace inodes of `pid` as seen from this process.
///
/// # Errors
///
/// Returns [`NamespaceError::Read`] when the process is gone or procfs is unavailable.
pub fn namespace_ids_of(pid: i32) -> Result<NamespaceIds, NamespaceError> {
    namespace_ids(&pid.to_string())
}

/// This process's own namespace inodes.
///
/// # Errors
///
/// Returns [`NamespaceError::Read`] when procfs is unavailable.
pub fn own_namespace_ids() -> Result<NamespaceIds, NamespaceError> {
    namespace_ids("self")
}

/// Interface names in `pid`'s network namespace, read from `/proc/<pid>/net/dev`.
///
/// # Errors
///
/// Returns [`NamespaceError::Read`] when the process is gone or its namespace is unreadable.
pub fn interfaces_of(pid: i32) -> Result<Vec<String>, NamespaceError> {
    let table = fs::read_to_string(format!("/proc/{pid}/net/dev")).map_err(|error| {
        NamespaceError::Read {
            what: "net/dev",
            errno: errno_of(&error),
        }
    })?;
    Ok(table
        .lines()
        .skip(2)
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim().to_owned()))
        .collect())
}

/// Proves the network namespace holds only `lo` plus at most `tap_count` transferred endpoints.
///
/// # Errors
///
/// Returns [`NamespaceError::UnexpectedInterfaces`] otherwise.
pub fn verify_interfaces(pid: i32, tap_count: usize) -> Result<usize, NamespaceError> {
    let interfaces = interfaces_of(pid)?;
    let extra = interfaces.iter().filter(|name| *name != "lo").count();
    if extra > tap_count {
        return Err(NamespaceError::UnexpectedInterfaces {
            found: interfaces.len(),
            allowed: tap_count + 1,
        });
    }
    Ok(interfaces.len())
}

/// Reads capabilities, `NoNewPrivs`, seccomp mode, and identity of `pid`.
///
/// # Errors
///
/// Returns [`NamespaceError::Read`] or [`NamespaceError::Malformed`].
pub fn process_status(pid: i32) -> Result<ProcessStatus, NamespaceError> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).map_err(|error| {
        NamespaceError::Read {
            what: "status",
            errno: errno_of(&error),
        }
    })?;
    let field = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .map(str::trim)
            .ok_or(NamespaceError::Malformed("status"))
    };
    let hex = |key: &str| {
        u64::from_str_radix(field(key)?, 16).map_err(|_| NamespaceError::Malformed("status"))
    };
    let first_number = |key: &str| {
        field(key)?
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or(NamespaceError::Malformed("status"))
    };
    Ok(ProcessStatus {
        capabilities_effective: hex("CapEff:")?,
        capabilities_permitted: hex("CapPrm:")?,
        capabilities_bounding: hex("CapBnd:")?,
        no_new_privs: first_number("NoNewPrivs:")? == 1,
        seccomp_mode: first_number("Seccomp:")?,
        uid: first_number("Uid:")?,
        gid: first_number("Gid:")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_namespaces_are_readable_when_procfs_exists() {
        if fs::metadata("/proc/self/ns/user").is_err() {
            return;
        }
        let ids = own_namespace_ids().expect("own namespace ids");
        assert!(ids.user != 0 && ids.mnt != 0 && ids.pid != 0);
        assert!(!ids.differs_entirely_from(&ids));
    }

    #[test]
    fn interface_table_parses_lo() {
        if fs::metadata("/proc/self/net/dev").is_err() {
            return;
        }
        let names = interfaces_of(std::process::id().try_into().expect("pid")).expect("interfaces");
        assert!(names.iter().any(|name| name == "lo"));
    }
}
