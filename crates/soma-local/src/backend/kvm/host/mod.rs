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

pub(super) mod channel;
mod serve;
mod transport;
mod wire;

#[cfg(test)]
mod tests;

use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use soma::{
    BackendFailureKind, CleanupEvidence, CommandStatus, EffectiveNetwork, FileAnswer,
    FileOperation, InstanceId, MachineShape, MachineState, OperationId, PtyAnswer, PtyOperation,
    SandboxLiveness,
};

pub(crate) use serve::host_machine;
use transport::{ask, await_close, exchange, open};
pub(in crate::backend::kvm) use wire::Launched;
use wire::{Answer, Call, LaunchWire, Ready};

/// Who holds the machines this Backend launches.
pub(super) enum Role {
    /// This process does, for as long as it runs. That is the one-shot lifecycle `soma run`
    /// needs, and it is measured without a second process anywhere in the path.
    Resident,
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

/// Starts the host that will hold one machine, and returns what its launch established.
///
/// The host is started before the machine exists, so there is never a moment at which a live
/// machine belongs to a process that is about to exit.
pub(super) fn launch(
    directory: &Path,
    operation_id: &OperationId,
    instance_id: &InstanceId,
    reference: String,
    shape: &MachineShape,
) -> Result<Launched, BackendFailureKind> {
    // Asked before anything is built, because a directory too deep to name a socket in cannot
    // hold a host for any Instance, and finding that out from a failed `bind` inside the host
    // reports a machine that would not start rather than a state root that cannot address one.
    if !channel::addressable(directory) {
        return Err(BackendFailureKind::Unsupported);
    }
    channel::prepare_directory(directory).map_err(|()| BackendFailureKind::Unavailable)?;
    let socket = channel::socket_path(directory, instance_id);
    let executable = std::env::current_exe().map_err(|_| BackendFailureKind::Unavailable)?;
    let mut child = Command::new(executable)
        .arg("machine-host")
        .arg(&socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // A host that inherited this process's error stream would hold a pipe its caller drains,
        // so a caller waiting for that stream to close would wait for the machine's whole life.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| BackendFailureKind::Unavailable)?;
    match handshake(&mut child, operation_id, instance_id, reference, shape) {
        Ok(launched) => Ok(launched),
        Err(kind) => {
            // Nothing was established, so no process may be left holding this Instance identity.
            let _ignored = child.kill();
            let _ignored = child.wait();
            Err(kind)
        }
    }
}

/// Hands the host its launch and reads the single line it answers with.
fn handshake(
    child: &mut Child,
    operation_id: &OperationId,
    instance_id: &InstanceId,
    reference: String,
    shape: &MachineShape,
) -> Result<Launched, BackendFailureKind> {
    let request = LaunchWire {
        operation_id: operation_id.clone(),
        instance_id: instance_id.clone(),
        reference,
        shape: shape.clone(),
    };
    let mut input = child.stdin.take().ok_or(BackendFailureKind::Unavailable)?;
    channel::write_line(&mut input, &request).map_err(|()| BackendFailureKind::Unavailable)?;
    // Closing the input is what tells the host that its launch has been stated in full.
    drop(input);
    let output = child.stdout.take().ok_or(BackendFailureKind::Unavailable)?;
    let mut reader = BufReader::new(output);
    // The reply is read here and the host writes nothing afterwards, so this pipe closes when
    // this reader is dropped rather than at the end of the machine's life.
    match channel::read_line::<Ready>(&mut reader) {
        Some(Ready::Launched(launched)) => Ok(launched),
        Some(Ready::Refused(refusal)) => Err(refusal.into()),
        None => Err(BackendFailureKind::Unavailable),
    }
}

pub(super) fn execute(
    directory: &Path,
    instance: &InstanceId,
    program: Vec<u8>,
    arguments: Vec<Vec<u8>>,
    timeout_ms: u32,
    max_output_bytes: u64,
) -> Result<Executed, HostFailure> {
    let call = Call::Execute {
        instance_id: instance.clone(),
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
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
