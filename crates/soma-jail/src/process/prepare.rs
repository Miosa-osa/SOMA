//! Parent-side preparation: the descriptor sources, the two launcher pipes, and the plan the
//! child executes without allocating.

#![allow(unsafe_code)]

use std::{
    ffi::CString,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::ffi::OsStrExt,
    path::Path,
};

use super::{child::ChildPlan, launch_error::LaunchError};
use crate::{
    descriptors::{SealPlan, launcher_sealed_len, report_slot},
    manifest::DescriptorRole,
    seccomp::{Phase, program_for, to_sock_filters},
    spec::JailSpec,
};

/// The already-open resources transferred to the child, in manifest order.
#[derive(Debug)]
pub struct Resources {
    /// Standard input; expected to be `/dev/null`.
    pub null: OwnedFd,
    /// Standard output and error.
    pub log: OwnedFd,
    /// The content-addressed VMM executable, opened read-only.
    pub executable: OwnedFd,
    pub descriptors: Vec<(DescriptorRole, OwnedFd)>,
}

/// The ends the parent keeps after the clone.
pub(super) struct ParentEnds {
    /// Writing one byte releases the child past its identity change.
    pub sync_write: OwnedFd,
    /// Delivers the twelve-byte failure report or EOF after a successful `execveat`.
    pub report_read: OwnedFd,
}

/// The child plan and the parent's ends; dropping the plan closes every child-side end.
pub(super) struct Launchpad {
    pub plan: ChildPlan,
    pub ends: ParentEnds,
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn pipe() -> Result<(OwnedFd, OwnedFd), LaunchError> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is valid storage for two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(LaunchError::Pipe(errno()));
    }
    // SAFETY: both descriptors were just created and are owned by nobody else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn relocate(fd: &OwnedFd, floor: u32) -> Result<OwnedFd, LaunchError> {
    let floor = libc::c_int::try_from(floor).map_err(|_| LaunchError::Pipe(libc::EINVAL))?;
    // SAFETY: `F_DUPFD_CLOEXEC` duplicates an owned descriptor and touches no memory.
    let relocated = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, floor) };
    if relocated < 0 {
        return Err(LaunchError::Pipe(errno()));
    }
    // SAFETY: the kernel just returned this descriptor and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(relocated) })
}

/// Builds the seal plan, the pipes, the startup filter, and the `argv` before the clone.
pub(super) fn prepare(
    spec: &JailSpec,
    jail_root: &Path,
    resources: &Resources,
) -> Result<Launchpad, LaunchError> {
    let (sync_read, sync_write) = pipe()?;
    let (report_read, report_write) = pipe()?;
    let sealed_len = launcher_sealed_len(&spec.manifest);
    let mut sources = vec![
        (0, &resources.null, false),
        (1, &resources.log, false),
        (2, &resources.log, false),
    ];
    for (index, (_, descriptor)) in resources.descriptors.iter().enumerate() {
        sources.push((spec.manifest.slot_of(index), descriptor, false));
    }
    sources.push((spec.manifest.executable_slot(), &resources.executable, true));
    sources.push((report_slot(&spec.manifest), &report_write, true));
    let seal = SealPlan::new(sealed_len, sources).map_err(LaunchError::Seal)?;
    let sync_read_high = relocate(&sync_read, sealed_len)?;
    let report_before_seal = seal
        .entries()
        .iter()
        .find(|entry| entry.slot == report_slot(&spec.manifest))
        .map_or(-1, |entry| entry.source);
    let plan = ChildPlan {
        identity: spec.identity,
        rlimits: spec.rlimits,
        root: CString::new(jail_root.as_os_str().as_bytes()).unwrap_or_default(),
        sync_read: sync_read_high.as_raw_fd(),
        report_before_seal,
        report_slot: libc::c_int::try_from(report_slot(&spec.manifest)).unwrap_or(-1),
        seal,
        manifest: spec.manifest.clone(),
        filter: to_sock_filters(&program_for(Phase::Startup)),
        executable_slot: libc::c_int::try_from(spec.manifest.executable_slot()).unwrap_or(-1),
        arguments: [
            c"soma-vmm".to_owned(),
            CString::new(spec.manifest.encode()).unwrap_or_default(),
        ],
        _child_ends: vec![sync_read_high, sync_read, report_write],
    };
    Ok(Launchpad {
        plan,
        ends: ParentEnds {
            sync_write,
            report_read,
        },
    })
}
