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
    io::BufReader,
    path::{Path, PathBuf},
    time::Duration,
};

use soma::{
    BackendFailureKind, CleanupEvidence, CommandStatus, EffectiveNetwork, FileAnswer,
    FileOperation, InstanceId, MachineState, PtyAnswer, PtyOperation, SandboxLiveness,
};

pub(crate) use bootstrap::host_machine;
pub(super) use client_launch::launch;
pub(crate) use sterile::{PrewarmPlan, prewarm as prewarm_machine_hosts};
use transport::{ask, await_close, exchange, open};
pub(crate) use tunnel::open_tunnel;
pub(in crate::backend::kvm) use wire::Launched;
use wire::{Answer, Call};

use super::evidence::CommandParts;

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

/// Claims the host that will hold one machine, and returns what its launch established.
///
/// The host process exists before the machine, so a live machine never belongs to a process that
/// is about to exit.
pub(super) fn execute(
    directory: &Path,
    instance: &InstanceId,
    parts: CommandParts,
) -> Result<Executed, HostFailure> {
    let CommandParts {
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
        context,
    } = parts;
    let call = if context.is_empty() {
        Call::Execute {
            instance_id: instance.clone(),
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
        }
    } else {
        Call::ExecuteInContext {
            instance_id: instance.clone(),
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
            context,
        }
    };
    match ask(directory, instance, &call, EXECUTE_CEILING)? {
        Answer::Executed {
            status,
            stdout,
            stderr,
        } => Ok(Executed {
            status,
            stdout,
            stderr,
        }),
        Answer::Refused(refusal) => Err(HostFailure::Refused(refusal.into())),
        _ => Err(HostFailure::Refused(BackendFailureKind::GuestFailure)),
    }
}

/// Runs a command on the hosted machine, delivering each output chunk to `sink` as the host
/// forwards it.
///
/// The read timeout applies to each line, so a command that stays silent longer than the ceiling
/// fails the way a buffered command that outlives it does. The chunks are checked against the
/// byte counts the final line names; a host that sends more or fewer is not trusted.
pub(super) fn execute_streaming(
    directory: &Path,
    instance: &InstanceId,
    parts: CommandParts,
    sink: soma::OutputSink<'_>,
) -> Result<Executed, HostFailure> {
    let CommandParts {
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
        context,
    } = parts;
    let call = Call::ExecuteStreaming {
        instance_id: instance.clone(),
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
        context,
    };
    let unavailable = || HostFailure::Refused(BackendFailureKind::Unavailable);
    let mut stream = open(directory, instance, EXECUTE_CEILING)?;
    channel::write_line(&mut stream, &call).map_err(|()| unavailable())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|_| unavailable())?);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    loop {
        match channel::read_line::<Answer>(&mut reader).ok_or_else(unavailable)? {
            Answer::Chunk { channel, bytes } => {
                if (stdout.len() + stderr.len() + bytes.len()) as u64 > max_output_bytes {
                    return Err(HostFailure::Refused(BackendFailureKind::GuestFailure));
                }
                let channel = soma::OutputChannel::from(channel);
                sink(channel, &bytes);
                match channel {
                    soma::OutputChannel::Stdout => stdout.extend_from_slice(&bytes),
                    soma::OutputChannel::Stderr => stderr.extend_from_slice(&bytes),
                }
            }
            Answer::StreamEnded {
                status,
                stdout_bytes,
                stderr_bytes,
            } => {
                if stdout.len() as u64 != stdout_bytes || stderr.len() as u64 != stderr_bytes {
                    return Err(HostFailure::Refused(BackendFailureKind::GuestFailure));
                }
                return Ok(Executed {
                    status,
                    stdout,
                    stderr,
                });
            }
            Answer::Refused(refusal) => return Err(HostFailure::Refused(refusal.into())),
            _ => return Err(HostFailure::Refused(BackendFailureKind::GuestFailure)),
        }
    }
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

/// Releases the hosted machine, and waits for the host to close the connection.
///
/// The host releases everything it owns before it answers, and closes only on its way out, so a
/// close observed after the evidence is the point at which nothing of this Instance is left in
/// any process. The socket is then removed, because an Instance with no host must not leave a
/// name behind that a later lookup would report as one.
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
    let _ignored = std::fs::remove_file(channel::socket_path(directory, instance));
    Ok(evidence)
}
