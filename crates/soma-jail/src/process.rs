//! The launcher: records ownership, builds the cgroup leaf, clones the child into its
//! namespaces and cgroup, releases it after the identity maps exist, and returns a pidfd-owning
//! handle with parent-side evidence.

#![allow(unsafe_code)]

mod child;
mod failure;
mod handle;
mod launch_error;
mod prepare;
mod spawn;
mod wait;

use std::{
    fs, io,
    os::fd::AsRawFd,
    path::PathBuf,
    time::{Duration, Instant},
};

pub use failure::{ChildFailure, ChildStep, RlimitKind};
pub use handle::JailHandle;
pub use launch_error::{LaunchError, LaunchFailure};
pub use prepare::Resources;
pub use wait::{SignalError, WaitError};
pub(crate) use wait::{send_signal, wait_exit};

use self::{
    prepare::{Launchpad, ParentEnds, prepare},
    spawn::{clone_child, read_report},
};
use crate::{
    cgroup::{CgroupError, CgroupLeaf},
    evidence::{JailEvidence, NamespaceIds, ProcessStatus},
    manifest::{DescriptorRole, STANDARD_STREAMS},
    namespaces::{
        NamespaceError, namespace_ids_of, own_namespace_ids, process_status, verify_interfaces,
        write_id_maps,
    },
    reconcile::{Disposition, JailLedger},
    seccomp::Phase,
    spec::JailSpec,
};

const CLEANUP_DEADLINE: Duration = Duration::from_secs(5);
const CHILD_REPORT_DEADLINE: Duration = Duration::from_secs(10);
const STATUS_SETTLE: Duration = Duration::from_millis(250);

/// Privileged-side locations that never reach the child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostAnchors {
    /// A cgroup2 mount or delegated subtree with `cpu`, `memory`, and `pids` in
    /// `cgroup.subtree_control`.
    pub cgroup_root: PathBuf,
    /// Where the empty jail-root directory is created, one per leaf name.
    pub jail_root_parent: PathBuf,
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Identity maps, namespace evidence, interface count, and cgroup membership before release.
fn observe(
    spec: &JailSpec,
    pid: i32,
    ledger: &JailLedger,
) -> Result<(NamespaceIds, usize), LaunchError> {
    write_id_maps(pid, spec.identity).map_err(LaunchError::IdMap)?;
    let own = own_namespace_ids().map_err(LaunchError::Namespace)?;
    let namespaces = namespace_ids_of(pid).map_err(LaunchError::Namespace)?;
    if !namespaces.differs_entirely_from(&own) {
        return Err(LaunchError::Namespace(NamespaceError::NotIsolated));
    }
    let tap_count = spec
        .manifest
        .roles()
        .iter()
        .filter(|role| **role == DescriptorRole::Tap)
        .count();
    let interface_count = verify_interfaces(pid, tap_count).map_err(LaunchError::Namespace)?;
    let member = ledger.cgroup().map_or(Ok(false), |leaf| leaf.contains(pid));
    match member {
        Ok(true) => Ok((namespaces, interface_count)),
        Ok(false) => Err(LaunchError::CgroupMembership(CgroupError::NotMember(pid))),
        Err(error) => Err(LaunchError::CgroupMembership(error)),
    }
}

/// Releases the child, waits for `execveat`, and confirms the post-exec status.
///
/// The report pipe reaches EOF when exec closes it, a moment before the kernel commits the
/// post-exec credentials, so the status read retries briefly until it settles.
fn release(
    spec: &JailSpec,
    pid: i32,
    ends: &ParentEnds,
    ledger: &mut JailLedger,
) -> Result<ProcessStatus, LaunchError> {
    // SAFETY: one byte from a valid local buffer is written to an owned descriptor.
    if unsafe { libc::write(ends.sync_write.as_raw_fd(), [1u8].as_ptr().cast(), 1) } != 1 {
        return Err(LaunchError::Pipe(errno()));
    }
    let deadline = Instant::now() + CHILD_REPORT_DEADLINE;
    if let Some(failure) = read_report(&ends.report_read, deadline)? {
        return Err(LaunchError::Child(failure));
    }
    let settle = Instant::now() + STATUS_SETTLE;
    loop {
        let status = match process_status(pid) {
            Ok(status) => status,
            Err(error) => return Err(died_or(ledger, LaunchError::Namespace(error))),
        };
        let expected = status.uid == spec.identity.uid
            && status.gid == spec.identity.gid
            && status.no_new_privs
            && status.seccomp_mode == 2
            && status.capabilities_effective == 0
            && status.capabilities_permitted == 0;
        if expected {
            return Ok(status);
        }
        if Instant::now() >= settle {
            return Err(died_or(ledger, LaunchError::Status(status)));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Replaces `error` with the child's exit reason when it has already died.
fn died_or(ledger: &mut JailLedger, error: LaunchError) -> LaunchError {
    let Some(pidfd) = ledger.pidfd() else {
        return error;
    };
    match wait_exit(pidfd, Instant::now() + STATUS_SETTLE) {
        Ok(exit) => {
            ledger.record_reaped();
            LaunchError::Died(exit)
        }
        Err(_) => error,
    }
}

/// Builds the jail around `resources` and returns once the VMM has executed inside it.
///
/// # Errors
///
/// Returns a [`LaunchFailure`] carrying the typed error and the cleanup disposition of
/// everything created before the failure.
pub fn launch(
    spec: &JailSpec,
    anchors: &HostAnchors,
    resources: Resources,
) -> Result<JailHandle, LaunchFailure> {
    let unstarted = |error: LaunchError| LaunchFailure {
        error,
        cleanup: Disposition::Released,
    };
    spec.validate()
        .map_err(|error| unstarted(LaunchError::Spec(error)))?;
    let roles: Vec<DescriptorRole> = resources
        .descriptors
        .iter()
        .map(|(role, _)| *role)
        .collect();
    if roles != spec.manifest.roles() {
        return Err(unstarted(LaunchError::ManifestMismatch));
    }
    let jail_root = anchors.jail_root_parent.join(spec.leaf.as_str());
    let mut ledger = JailLedger::new(spec.leaf.as_str().to_owned(), jail_root.clone());
    let fail = |ledger: &mut JailLedger, error: LaunchError| LaunchFailure {
        error,
        cleanup: ledger.reconcile(Instant::now() + CLEANUP_DEADLINE),
    };

    let leaf = match CgroupLeaf::create(&anchors.cgroup_root, &spec.leaf, &spec.limits) {
        Ok(leaf) => leaf,
        Err(error) => return Err(fail(&mut ledger, LaunchError::Cgroup(error))),
    };
    ledger.record_cgroup(leaf);
    if let Err(error) = fs::create_dir(&jail_root) {
        return Err(fail(
            &mut ledger,
            LaunchError::JailRoot(error.raw_os_error().unwrap_or(0)),
        ));
    }
    ledger.record_jail_root();
    let Launchpad { plan, ends } = match prepare(spec, &jail_root, &resources) {
        Ok(launchpad) => launchpad,
        Err(error) => return Err(fail(&mut ledger, error)),
    };
    let cloned = ledger.cgroup().map(|leaf| clone_child(&plan, leaf));
    // Every child-side end and relocated source closes here so the report pipe can reach EOF.
    drop(plan);
    drop(resources);
    let (pid, pidfd) = match cloned {
        Some(Ok(child)) => child,
        Some(Err(error)) => return Err(fail(&mut ledger, error)),
        None => return Err(fail(&mut ledger, LaunchError::Clone(libc::EINVAL))),
    };
    ledger.record_child(pid, pidfd);

    let (namespaces, interface_count) = match observe(spec, pid, &ledger) {
        Ok(observed) => observed,
        Err(error) => return Err(fail(&mut ledger, error)),
    };
    let status = match release(spec, pid, &ends, &mut ledger) {
        Ok(status) => status,
        Err(error) => return Err(fail(&mut ledger, error)),
    };
    drop(ends);
    let evidence = JailEvidence {
        identity: spec.identity,
        namespaces,
        status,
        leaf: spec.leaf.as_str().to_owned(),
        descriptor_count: STANDARD_STREAMS
            + u32::try_from(spec.manifest.roles().len()).unwrap_or(u32::MAX),
        filter_phase: Phase::Startup,
        interface_count,
        exit: None,
        oom_kills: 0,
    };
    Ok(JailHandle::new(ledger, pid, evidence))
}
