//! The process that holds one machine.
//!
//! A sterile process waits without an Instance or machine, receives one complete launch
//! capability, binds that Instance's socket, launches exactly one machine, and reports what that
//! established on standard output.
//! It then answers one request at a time until the machine is released.
//!
//! It ends by releasing. Every path out of the serve loop passes through cleanup, so a client
//! that vanished, a host that was told to shut down, and a listener that failed all leave the
//! same thing behind: no machine, no overlay head, no lease, and no socket.

mod requests;

use std::{
    io,
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

use crate::backend::kvm::{
    KvmBackend, held,
    lifecycle::{Force, LaunchExtras},
    prepared,
};

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

pub(super) fn serve_machine(
    listener: &UnixListener,
    socket: &Path,
    request: &LaunchWire,
    prepared: &prepared::PreparedGeneration,
    mut backend: KvmBackend,
) -> i32 {
    let instance = request.instance_id.clone();
    // The launch is timed from the moment this process received it, not from the first milestone
    // that reads the clock: everything between is work this launch asked for, and a parent that
    // added its own elapsed time before the handoff would otherwise lose all of it.
    backend.begin_operation(&request.operation_id);
    // Claimed: leave the API unit's control group, so stopping or restarting the API does not kill
    // this machine. The Instance socket is already bound, so a restarted API finds it by identity.
    let joined = super::live_cgroup::join(&instance);
    let mut launched = match launch(&mut backend, request, prepared) {
        Ok(launched) => launched,
        Err(kind) => return refuse(kind),
    };
    launched.live_cgroup = joined.report();
    if channel::write_line(&mut io::stdout(), &Ready::Launched(launched)).is_err() {
        // The client is gone and will never address this Instance, so the machine it asked for
        // must not outlive the answer it never received.
        let _ignored = backend.cleanup_resident(&instance, Force::Immediately);
        return REFUSED;
    }
    // After the launch has answered, so the create pays nothing for it, and before the first
    // request is accepted, so no request is ever interleaved with it.
    if !request.skip_warm {
        super::warm::warm(&mut backend, &instance);
    }
    answer_until_released(listener, socket, &mut backend, &instance)
}

/// Builds the one machine this host will hold.
///
/// The prepared entry is found here rather than handed over, so what launches is what an entry
/// in this host's own store claims for the reference, read by exactly the check the one-shot
/// path performs.
fn launch(
    backend: &mut KvmBackend,
    request: &LaunchWire,
    found: &prepared::PreparedGeneration,
) -> Result<Launched, BackendFailureKind> {
    backend
        .launch_resident(
            &request.operation_id,
            &request.instance_id,
            found,
            &request.shape,
            LaunchExtras {
                volume: request.volume.clone(),
                windowed: request.windowed,
            },
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
        let Some(call) = requests::receive(&mut stream) else {
            continue;
        };
        if matches!(call, Call::Shutdown) {
            break;
        }
        if let Call::Tunnel { instance_id, port } = &call {
            // The caller's connection becomes the tunnel, so it is handed over and the loop
            // moves on to the next request.
            super::tunnel::serve(backend, stream, (instance_id, *port), &activity, started);
            activity.store(started.elapsed().as_secs(), Ordering::Relaxed);
            continue;
        }
        if let Call::ExecuteStreaming {
            instance_id,
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
            context,
        } = call
        {
            let answer = stream_command(
                backend,
                &mut stream,
                &instance_id,
                (program, arguments, timeout_ms, max_output_bytes),
                &context,
            );
            let _ignored = channel::write_line(&mut stream, &answer);
            activity.store(started.elapsed().as_secs(), Ordering::Relaxed);
            continue;
        }
        if let Call::Cleanup {
            instance_id,
            forced,
        } = &call
        {
            let answer = requests::release(backend, instance_id, requests::force_for(*forced));
            let released = matches!(answer, Answer::Cleaned { .. });
            let _ignored = channel::write_line(&mut stream, &answer);
            if released {
                // The caller waits for this connection to close. It closes here, once the guest
                // can no longer run, rather than when the VM's memory has been returned.
                drop(stream);
                held::finish_released();
                return RELEASED;
            }
            continue;
        }
        let answer = requests::perform(backend, call);
        let _ignored = channel::write_line(&mut stream, &answer);
        activity.store(started.elapsed().as_secs(), Ordering::Relaxed);
    }
    // Nothing more is coming, so this host releases what it still holds rather than keeping a
    // machine alive that no process can reach.
    let _ignored = backend.cleanup_resident(instance, Force::Immediately);
    held::finish_released();
    RELEASED
}

/// Runs one streaming command, writing each chunk to the caller's connection as it arrives, and
/// returns the one final answer.
///
/// A caller that stops reading cannot abort the guest exchange half way, so the failed write is
/// remembered, further chunks are dropped, and the command still ends and is recorded.
fn stream_command(
    backend: &mut KvmBackend,
    stream: &mut UnixStream,
    instance_id: &InstanceId,
    (program, arguments, timeout_ms, max_output_bytes): (Vec<u8>, Vec<Vec<u8>>, u32, u64),
    context: &crate::backend::kvm::evidence::WireContext,
) -> Answer {
    let Ok(command) = GuestCommand::new(program, arguments, timeout_ms, max_output_bytes)
        .and_then(|command| context.clone().apply(command))
    else {
        return Answer::Refused(BackendFailureKind::WorkloadRejected.into());
    };
    let mut caller_gone = false;
    let mut sink = |channel: soma::OutputChannel, bytes: &[u8]| {
        if caller_gone {
            return;
        }
        let chunk = Answer::Chunk {
            channel: channel.into(),
            bytes: bytes.to_vec(),
        };
        if channel::write_line(stream, &chunk).is_err() {
            caller_gone = true;
        }
    };
    match backend.execute_resident_streaming(instance_id, command, &mut sink) {
        Ok((status, stdout, stderr)) => Answer::StreamEnded {
            status,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: stderr.len() as u64,
        },
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
pub(super) fn refuse(kind: BackendFailureKind) -> i32 {
    let _ignored = channel::write_line(&mut io::stdout(), &Ready::Refused(kind.into()));
    REFUSED
}
