//! A machine that outlives the process which asked for it.
//!
//! A KVM sandbox is a set of descriptors, a guest memory mapping, a vCPU thread, and an
//! authenticated session. All four belong to one process, so a machine can only survive its
//! launching command if some other process is holding them. That process is what this module
//! starts and addresses.
//!
//! One host serves one Instance for its whole life. It binds the socket named by that Instance
//! before it builds anything, launches exactly the machine the client asked for, reports the
//! facts of that launch on its standard output, and then answers execute, inspect, and cleanup
//! over the socket until the machine is released. Nothing about the machine changes: the host
//! runs the same resident lifecycle the one-shot path runs, so the per-Instance identity, the
//! sterile assignment, and the Noise session are established exactly once, by the process that
//! holds them.

mod bootstrap;
pub(super) mod channel;
mod client_launch;
mod command;
mod live_cgroup;
mod reaper;
mod serve;
mod sterile;
mod transport;
mod tunnel;
mod warm;
mod warm_gate;
mod wire;

#[cfg(test)]
mod tests;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use soma::{
    BackendFailureKind, CleanupEvidence, CommandStatus, EffectiveNetwork, FileAnswer,
    FileOperation, InstanceId, MachineState, PtyAnswer, PtyOperation, SandboxLiveness,
};

pub(crate) use bootstrap::host_machine;
pub(in crate::backend) use client_launch::LaunchHandoff;
pub(super) use client_launch::{HostedLaunch, launch};
pub(super) use command::{execute, execute_streaming};
pub(in crate::backend::kvm) use live_cgroup::under_root_lock;
pub(crate) use sterile::{
    PrewarmPlan, TargetPlan, install_sets, prewarm as prewarm_machine_hosts, snapshot,
};
use transport::{ask, await_close, exchange, open};
pub(crate) use tunnel::open_tunnel;
use wire::{Answer, Call};
pub(in crate::backend::kvm) use wire::{HeadSpanWire, Launched};

/// Who holds the machines this Backend launches.
pub(super) enum Role {
    /// This process does, for as long as it runs. That is the one-shot lifecycle `soma run`
    /// needs, and it is measured without a second process anywhere in the path.
    Resident,
    /// A process that holds exactly one requested Instance and exits with it.
    MachineHost,
    /// A host process does, addressed by Instance identity under this directory.
    Hosted(PathBuf),
}

/// How long a caller waits for the answer to one bounded command.
///
/// It exceeds the ceiling the host itself puts on a command, so a host that answers late is
/// still heard rather than reported as broken by the process that asked it.
const EXECUTE_CEILING: Duration = Duration::from_secs(330);
/// How long a caller waits for an inspection or a release.
const CONTROL_CEILING: Duration = Duration::from_secs(120);
/// How long a caller waits for one terminal operation.
///
/// A read states the longest it will wait for its first byte, so the ceiling is that bound with
/// room for the exchange around it.
const PTY_CEILING: Duration = Duration::from_millis(soma::MAX_PTY_WAIT_MILLIS as u64 + 60_000);

/// One completed command as it crossed back from the host.
pub(super) struct Executed {
    pub(super) status: CommandStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

/// Why an operation against a hosted Instance produced no answer.
#[derive(Clone, Copy)]
pub(super) enum HostFailure {
    /// No host here serves this Instance.
    Absent,
    /// A host answered with a refusal, or the exchange with it broke.
    Refused(BackendFailureKind),
}

/// Asks the host holding this Instance for one bounded filesystem operation.
///
/// A whole-file transfer is several guest records, so it is given the command ceiling rather
/// than the control one: it can take about as long as a command and for the same reason, which
/// is that the guest is doing real work for the whole of it.
pub(super) fn file(
    directory: &Path,
    instance: &InstanceId,
    operation: &FileOperation,
) -> Result<FileAnswer, HostFailure> {
    let call = Call::File {
        instance_id: instance.clone(),
        operation: operation.clone(),
    };
    match ask(directory, instance, &call, EXECUTE_CEILING)? {
        Answer::FileAnswered { answer } => Ok(answer),
        Answer::Refused(refusal) => Err(HostFailure::Refused(refusal.into())),
        _ => Err(HostFailure::Refused(BackendFailureKind::GuestFailure)),
    }
}

/// Asks the host holding this Instance for one bounded terminal operation.
///
/// The control ceiling is not enough here: a read may ask the guest to wait a minute for its
/// first byte, and the caller has to outwait what it asked for or it would report a host as
/// broken for doing exactly what the request said.
pub(super) fn pty(
    directory: &Path,
    instance: &InstanceId,
    operation: &PtyOperation,
) -> Result<PtyAnswer, HostFailure> {
    let call = Call::Pty {
        instance_id: instance.clone(),
        operation: operation.clone(),
    };
    match ask(directory, instance, &call, PTY_CEILING)? {
        Answer::PtyAnswered { answer } => Ok(answer),
        Answer::Refused(refusal) => Err(HostFailure::Refused(refusal.into())),
        _ => Err(HostFailure::Refused(BackendFailureKind::GuestFailure)),
    }
}

/// Reports whether a host process is serving this Instance right now.
///
/// It connects and does nothing else. What that proves is exactly one thing and it is worth
/// naming: a process has the Instance's socket bound and accepted a connection on it. It is not
/// a claim that the guest inside is healthy, which is what an inspection by exact identity
/// answers; it is the difference between a durable record whose host is running and one whose
/// host is gone.
///
/// A socket nothing answers on is removed by the connect itself, so a host that died leaves no
/// name behind for a second listing to report.
/// Sends an attachment or detachment for the owning process to serialize.
pub(crate) fn workspace(
    directory: &Path,
    instance: &InstanceId,
    request: &soma_hostd::Request,
) -> Result<soma_hostd::Reply, BackendFailureKind> {
    let request = Call::Workspace {
        instance_id: instance.clone(),
        request: request.encode(),
    };
    match ask(directory, instance, &request, CONTROL_CEILING) {
        Ok(Answer::WorkspaceVolume(reply)) => {
            soma_hostd::Reply::decode(&reply).map_err(|_| BackendFailureKind::GuestFailure)
        }
        Ok(Answer::Refused(kind)) => Err(kind.into()),
        Err(HostFailure::Refused(kind)) => Err(kind),
        Ok(_) => Err(BackendFailureKind::GuestFailure),
        Err(HostFailure::Absent) => Err(BackendFailureKind::Unavailable),
    }
}

/// Asks the owning host to confirm that guest execution is paused.
pub(crate) fn pause(directory: &Path, instance: &InstanceId) -> Result<(), crate::PauseFailure> {
    let call = Call::Pause {
        instance_id: instance.clone(),
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::Paused) => Ok(()),
        Ok(Answer::PauseRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::PauseFailure::NotFound),
        _ => Err(crate::PauseFailure::Unavailable),
    }
}

/// Asks the owning host to confirm that guest execution is resumed.
pub(crate) fn resume(directory: &Path, instance: &InstanceId) -> Result<(), crate::ResumeFailure> {
    let call = Call::Resume {
        instance_id: instance.clone(),
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::Resumed) => Ok(()),
        Ok(Answer::ResumeRefused(failure)) => {
            eprintln!("soma: owning host refused resident resume: {failure:?}");
            Err(failure)
        }
        Err(HostFailure::Absent) => Err(crate::ResumeFailure::NotFound),
        Err(HostFailure::Refused(kind)) => {
            eprintln!("soma: resident resume host exchange failed: {kind:?}");
            Err(crate::ResumeFailure::Unavailable)
        }
        Ok(_) => {
            eprintln!("soma: owning host returned an unexpected resume receipt");
            Err(crate::ResumeFailure::Unavailable)
        }
    }
}

/// Asks the owning host to durably checkpoint its running Instance.
pub(crate) fn checkpoint(
    directory: &Path,
    instance: &InstanceId,
    operation: &soma::OperationId,
) -> Result<crate::CheckpointReceipt, crate::CheckpointFailure> {
    let call = Call::Checkpoint {
        instance_id: instance.clone(),
        operation_id: operation.clone(),
    };
    let ceiling = super::checkpoint::CHECKPOINT_CEILING + CONTROL_CEILING;
    match ask(directory, instance, &call, ceiling) {
        Ok(Answer::Checkpointed(receipt))
            if &receipt.instance_id == instance && &receipt.checkpoint_id == operation =>
        {
            Ok(receipt)
        }
        Ok(Answer::CheckpointRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::CheckpointFailure::NotFound),
        _ => Err(crate::CheckpointFailure::Unconfirmed),
    }
}

/// Asks the host that holds `instance` to change its CPU and memory live.
pub(crate) fn resize(
    directory: &Path,
    instance: &InstanceId,
    request: crate::ResizeRequest,
) -> Result<crate::ResizeOutcome, crate::ResizeFailure> {
    let call = Call::Resize {
        instance_id: instance.clone(),
        request,
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::Resized(outcome)) => Ok(outcome),
        Ok(Answer::ResizeRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::ResizeFailure::NotFound),
        _ => Err(crate::ResizeFailure::Unavailable),
    }
}

/// Asks the host that holds `instance` to open or close its egress.
///
/// Anything that is not the host's own outcome is a failure the caller must read as "the egress
/// may not be what I asked for", including a host that does not know the call.
pub(crate) fn set_egress(
    directory: &Path,
    instance: &InstanceId,
    request: crate::EgressSetRequest,
) -> Result<crate::EgressSetOutcome, crate::EgressSetFailure> {
    let call = Call::SetEgress {
        instance_id: instance.clone(),
        request,
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::EgressSet(outcome)) => Ok(outcome),
        Ok(Answer::EgressRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::EgressSetFailure::NotFound),
        _ => Err(crate::EgressSetFailure::Unavailable),
    }
}

pub(super) fn liveness(directory: &Path, instance: &InstanceId) -> SandboxLiveness {
    match channel::connect(directory, instance) {
        Ok(_) => SandboxLiveness::Live,
        Err(()) => SandboxLiveness::Absent,
    }
}

pub(super) fn inspect(
    directory: &Path,
    instance: &InstanceId,
) -> Result<(MachineState, EffectiveNetwork), HostFailure> {
    let call = Call::Inspect {
        instance_id: instance.clone(),
    };
    match ask(directory, instance, &call, CONTROL_CEILING)? {
        Answer::Inspected { state, network } => Ok((state, network)),
        Answer::Refused(refusal) => Err(HostFailure::Refused(refusal.into())),
        _ => Err(HostFailure::Refused(BackendFailureKind::Unavailable)),
    }
}

/// How long a suspension may take: reading every byte of guest memory and the private head.
const SUSPEND_CEILING: Duration = Duration::from_mins(11);

/// Asks the host holding `instance` to suspend it into `staging`; `false` is a refusal.
pub(super) fn suspend(
    directory: &Path,
    instance: &InstanceId,
    staging: std::path::PathBuf,
) -> Result<bool, HostFailure> {
    let call = Call::Suspend {
        instance_id: instance.clone(),
        staging,
    };
    match ask(directory, instance, &call, SUSPEND_CEILING)? {
        Answer::Suspended => Ok(true),
        Answer::SuspendRefused => Ok(false),
        Answer::Refused(refusal) => Err(HostFailure::Refused(refusal.into())),
        _ => Err(HostFailure::Refused(BackendFailureKind::Unavailable)),
    }
}

/// Releases the hosted machine, and waits for the host to close the connection.
///
/// The host removes its socket and releases the guest before closing the connection.
pub(super) fn cleanup(
    directory: &Path,
    instance: &InstanceId,
    forced: bool,
) -> Result<CleanupEvidence, HostFailure> {
    let call = Call::Cleanup {
        instance_id: instance.clone(),
        forced,
    };
    let mut stream = open(directory, instance, CONTROL_CEILING)?;
    let evidence = match exchange(&mut stream, &call)? {
        Answer::Cleaned { evidence } => evidence,
        Answer::Refused(refusal) => return Err(HostFailure::Refused(refusal.into())),
        _ => return Err(HostFailure::Refused(BackendFailureKind::CleanupFailure)),
    };
    await_close(&mut stream)?;
    Ok(evidence)
}
