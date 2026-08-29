//! The pre-exec child: every step between `clone3` and `execveat`.
//!
//! The parent may be multithreaded and `clone3` runs no fork handlers, so nothing here
//! allocates; every buffer and string lives in the [`ChildPlan`] built before the clone.

#![allow(unsafe_code)]

use std::{
    ffi::{CString, c_char},
    io,
    os::fd::{OwnedFd, RawFd},
};

use super::failure::{ChildFailure, ChildStep, REPORT_BYTES, RlimitKind};
use crate::{
    descriptors::{SealPlan, VerificationDepth, verify_sealed_table},
    manifest::DescriptorManifest,
    namespaces::enter_empty_root,
    seccomp::install_sock_filters,
    spec::{Identity, Rlimits},
};

/// Everything the child needs, prepared by the parent.
pub(crate) struct ChildPlan {
    pub identity: Identity,
    pub rlimits: Rlimits,
    pub root: CString,
    /// Relocated read end of the synchronization pipe.
    pub sync_read: RawFd,
    /// Relocated write end of the report pipe, valid until the table is sealed.
    pub report_before_seal: RawFd,
    /// Slot of the report pipe after sealing.
    pub report_slot: RawFd,
    pub seal: SealPlan,
    pub manifest: DescriptorManifest,
    pub filter: Vec<libc::sock_filter>,
    pub executable_slot: libc::c_int,
    /// `["soma-vmm", <manifest encoding>]` kept alive for `argv`.
    pub arguments: [CString; 2],
    /// Pipe ends only the child uses; the parent closes them by dropping the plan.
    pub _child_ends: Vec<OwnedFd>,
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn check(step: ChildStep, result: libc::c_int) -> Result<(), ChildFailure> {
    if result == 0 {
        Ok(())
    } else {
        Err(ChildFailure {
            step,
            errno: errno(),
        })
    }
}

fn death_signal() -> Result<(), ChildFailure> {
    // SAFETY: `PR_SET_PDEATHSIG` takes integer arguments only.
    check(ChildStep::DeathSignal, unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0)
    })
}

fn wait_for_launcher(sync_read: RawFd) -> Result<(), ChildFailure> {
    let mut byte = 0u8;
    loop {
        // SAFETY: `byte` is one writable byte and the length is one.
        let read = unsafe { libc::read(sync_read, (&raw mut byte).cast(), 1) };
        match read {
            1 => return Ok(()),
            0 => {
                return Err(ChildFailure {
                    step: ChildStep::LauncherGone,
                    errno: 0,
                });
            }
            _ if errno() == libc::EINTR => {}
            _ => {
                return Err(ChildFailure {
                    step: ChildStep::LauncherGone,
                    errno: errno(),
                });
            }
        }
    }
}

fn set_identity(identity: Identity) -> Result<(), ChildFailure> {
    // SAFETY: both calls take integer identities only.
    unsafe {
        check(
            ChildStep::SetGid,
            libc::setresgid(identity.gid, identity.gid, identity.gid),
        )?;
        check(
            ChildStep::SetUid,
            libc::setresuid(identity.uid, identity.uid, identity.uid),
        )?;
    }
    Ok(())
}

/// glibc types resource identifiers differently from musl.
#[cfg(target_env = "gnu")]
type Resource = libc::__rlimit_resource_t;
#[cfg(not(target_env = "gnu"))]
type Resource = libc::c_int;

fn limit(kind: RlimitKind, resource: Resource, value: u64) -> Result<(), ChildFailure> {
    let rlimit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: `rlimit` is valid readable storage for the duration of the call.
    let result = unsafe { libc::setrlimit(resource, &raw const rlimit) };
    check(ChildStep::Rlimit(kind), result)
}

/// Every limit except `RLIMIT_NOFILE`, which waits until the table is sealed because
/// `pivot_root` and the seal itself still need descriptors above the inherited table.
fn apply_rlimits(rlimits: &Rlimits) -> Result<(), ChildFailure> {
    limit(RlimitKind::Core, libc::RLIMIT_CORE, 0)?;
    limit(
        RlimitKind::Nproc,
        libc::RLIMIT_NPROC,
        u64::from(rlimits.nproc),
    )?;
    limit(RlimitKind::Fsize, libc::RLIMIT_FSIZE, rlimits.fsize_bytes)?;
    if let Some(bytes) = rlimits.address_space_bytes {
        limit(RlimitKind::AddressSpace, libc::RLIMIT_AS, bytes)?;
    }
    Ok(())
}

fn steps(plan: &ChildPlan, report: &mut RawFd) -> Result<(), ChildFailure> {
    death_signal()?;
    wait_for_launcher(plan.sync_read)?;
    set_identity(plan.identity)?;
    // Credential changes clear the parent-death signal, so it is set again here.
    death_signal()?;
    // SAFETY: `PR_SET_DUMPABLE` takes integer arguments only.
    check(ChildStep::Dumpable, unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0)
    })?;
    apply_rlimits(&plan.rlimits)?;
    enter_empty_root(&plan.root).map_err(|(step, errno)| ChildFailure {
        step: ChildStep::Root(step),
        errno,
    })?;
    plan.seal.apply_in_child().map_err(|error| ChildFailure {
        step: ChildStep::Seal(error),
        errno: 0,
    })?;
    *report = plan.report_slot;
    limit(
        RlimitKind::Nofile,
        libc::RLIMIT_NOFILE,
        u64::from(plan.rlimits.nofile),
    )?;
    verify_sealed_table(
        &plan.manifest,
        VerificationDepth::Launcher,
        plan.rlimits.nofile,
    )
    .map_err(|error| ChildFailure {
        step: ChildStep::Verify(error),
        errno: 0,
    })?;
    install_sock_filters(&plan.filter).map_err(|error| ChildFailure {
        step: ChildStep::Seccomp(error),
        errno: 0,
    })?;
    let argv: [*const c_char; 3] = [
        plan.arguments[0].as_ptr(),
        plan.arguments[1].as_ptr(),
        std::ptr::null(),
    ];
    let envp: [*const c_char; 1] = [std::ptr::null()];
    // SAFETY: `argv` and `envp` are null-terminated arrays of valid C strings that outlive the
    // call, the path is an empty C string, and `AT_EMPTY_PATH` selects the descriptor.
    unsafe {
        libc::syscall(
            libc::SYS_execveat,
            plan.executable_slot,
            c"".as_ptr(),
            argv.as_ptr(),
            envp.as_ptr(),
            libc::AT_EMPTY_PATH,
        );
    }
    Err(ChildFailure {
        step: ChildStep::Exec,
        errno: errno(),
    })
}

/// Runs every child step and never returns; a failure is reported through the pipe.
pub(crate) fn run(plan: &ChildPlan) -> ! {
    let mut report = plan.report_before_seal;
    let failure = steps(plan, &mut report).err().unwrap_or(ChildFailure {
        step: ChildStep::Exec,
        errno: 0,
    });
    let bytes = failure.encode();
    // SAFETY: `bytes` is valid readable storage of exactly `REPORT_BYTES`.
    unsafe {
        libc::write(report, bytes.as_ptr().cast(), REPORT_BYTES);
        libc::_exit(127);
    }
}
