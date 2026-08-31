//! The process that holds one machine.
//!
//! It binds its socket before anything exists, so the Instance is addressable from the moment it
//! could be launched; it launches exactly one machine and reports what that established on its
//! standard output; and it then answers one request at a time until the machine is released.
//!
//! It ends by releasing. Every path out of the serve loop passes through cleanup, so a client
//! that vanished, a host that was told to shut down, and a listener that failed all leave the
//! same thing behind: no machine, no overlay head, no lease, and no socket.

use std::{
    io::{self, BufReader},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use soma::{BackendFailureKind, InstanceId};
use soma_guest::GuestCommand;

use crate::backend::kvm::{KvmBackend, prepared};

use super::{
    Launched, channel,
    wire::{Answer, Call, LaunchWire, Ready},
};

/// How long a host whose machine nobody is addressing stays alive.
///
/// A client that died between launch and destroy would otherwise leave a machine running for as
/// long as this host does, which is forever. The ceiling is generous enough that an idle agent
/// session is not cut off, and finite enough that an abandoned machine is not a permanent leak.
const IDLE_CEILING: Duration = Duration::from_mins(30);
/// How often the idle check runs.
const IDLE_TICK: Duration = Duration::from_secs(15);
/// How long the host waits for a caller that has connected to state its request.
const REQUEST_CEILING: Duration = Duration::from_secs(30);

const RELEASED: i32 = 0;
const REFUSED: i32 = 1;

/// Serves one machine at `socket` until it is released, and returns the process status.
pub(crate) fn host_machine(socket: &Path) -> i32 {
    let Ok(listener) = channel::bind(socket) else {
        return REFUSED;
    };
    let status = serve(&listener, socket);
    let _ignored = std::fs::remove_file(socket);
    status
}

fn serve(listener: &UnixListener, socket: &Path) -> i32 {
    let mut input = BufReader::new(io::stdin());
    let Some(request) = channel::read_line::<LaunchWire>(&mut input) else {
        return REFUSED;
    };
    let Ok(mut backend) = KvmBackend::resident() else {
        return refuse(BackendFailureKind::Unavailable);
    };
    let instance = request.instance_id.clone();
    let launched = match launch(&mut backend, &request) {
        Ok(launched) => launched,
        Err(kind) => return refuse(kind),
    };
    if channel::write_line(&mut io::stdout(), &Ready::Launched(launched)).is_err() {
        // The client is gone and will never address this Instance, so the machine it asked for
        // must not outlive the answer it never received.
        let _ignored = backend.cleanup_resident(&instance);
        return REFUSED;
    }
    answer_until_released(listener, socket, &mut backend, &instance)
}

/// Builds the one machine this host will hold.
///
/// The prepared entry is found here rather than handed over, so what launches is what an entry
/// in this host's own store claims for the reference, read by exactly the check the one-shot
/// path performs.
fn launch(backend: &mut KvmBackend, request: &LaunchWire) -> Result<Launched, BackendFailureKind> {
    let found = prepared::find(
        prepared::store_root().as_deref(),
        &request.reference,
        prepared::uncertified_allowed(),
    )
    .map_err(|_| BackendFailureKind::WorkloadRejected)?;
    backend
        .launch_resident(
            &request.operation_id,
            &request.instance_id,
            &found,
            &request.shape,
        )
        .map_err(|failure| failure.kind())
}

fn answer_until_released(
    listener: &UnixListener,
    socket: &Path,
    backend: &mut KvmBackend,
    instance: &InstanceId,
) -> i32 {
    let activity = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    watch_for_idleness(socket.to_path_buf(), Arc::clone(&activity), started);
    for accepted in listener.incoming() {
        let Ok(mut stream) = accepted else {
            break;
        };
        let Some(call) = receive(&mut stream) else {
            continue;
        };
        if matches!(call, Call::Shutdown) {
            break;
        }
        if let Call::Cleanup { instance_id } = &call {
            let answer = release(backend, instance_id);
            let released = matches!(answer, Answer::Cleaned { .. });
            let _ignored = channel::write_line(&mut stream, &answer);
            if released {
                return RELEASED;
            }
            continue;
        }
        let answer = perform(backend, call);
        let _ignored = channel::write_line(&mut stream, &answer);
        activity.store(started.elapsed().as_secs(), Ordering::Relaxed);
    }
    // Nothing more is coming, so this host releases what it still holds rather than keeping a
    // machine alive that no process can reach.
    let _ignored = backend.cleanup_resident(instance);
    RELEASED
}

fn receive(stream: &mut UnixStream) -> Option<Call> {
    stream.set_read_timeout(Some(REQUEST_CEILING)).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    channel::read_line::<Call>(&mut reader)
}

fn perform(backend: &mut KvmBackend, call: Call) -> Answer {
    match call {
        Call::Execute {
            instance_id,
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
        } => {
            let Ok(command) = GuestCommand::new(program, arguments, timeout_ms, max_output_bytes)
            else {
                return Answer::Refused(BackendFailureKind::WorkloadRejected.into());
            };
            match backend.execute_resident(&instance_id, command) {
                Ok((status, stdout, stderr)) => Answer::Executed {
                    status,
                    stdout,
                    stderr,
                },
                Err(kind) => Answer::Refused(kind.into()),
            }
        }
        Call::Inspect { instance_id } => match backend.inspect_resident(&instance_id) {
            Ok((state, network)) => Answer::Inspected { state, network },
            Err(kind) => Answer::Refused(kind.into()),
        },
        // Both terminal calls are decided by the loop rather than here, because each ends the
        // process and only one of them answers first.
        Call::Cleanup { .. } | Call::Shutdown => {
            Answer::Refused(BackendFailureKind::Unsupported.into())
        }
    }
}

fn release(backend: &mut KvmBackend, instance: &InstanceId) -> Answer {
    match backend.cleanup_resident(instance) {
        Ok(evidence) => Answer::Cleaned { evidence },
        Err(kind) => Answer::Refused(kind.into()),
    }
}

/// Ends a host whose machine nothing has addressed for [`IDLE_CEILING`].
///
/// The shutdown is asked for over the host's own socket rather than by reaching into the machine
/// from another thread, so the release still happens on the one thread that owns it.
fn watch_for_idleness(socket: PathBuf, activity: Arc<AtomicU64>, started: Instant) {
    let _ignored = thread::Builder::new()
        .name("soma-machine-idle".to_owned())
        .spawn(move || {
            loop {
                thread::sleep(IDLE_TICK);
                let idle_for = started
                    .elapsed()
                    .as_secs()
                    .saturating_sub(activity.load(Ordering::Relaxed));
                if idle_for < IDLE_CEILING.as_secs() {
                    continue;
                }
                if let Ok(mut stream) = UnixStream::connect(&socket) {
                    let _ignored = channel::write_line(&mut stream, &Call::Shutdown);
                }
                return;
            }
        });
}

/// Reports a launch that never produced a machine, on the one stream the client is reading.
fn refuse(kind: BackendFailureKind) -> i32 {
    let _ignored = channel::write_line(&mut io::stdout(), &Ready::Refused(kind.into()));
    REFUSED
}
