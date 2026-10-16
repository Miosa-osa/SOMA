//! The supervisor's side of a streaming command against a worker that is only a socket.

use std::{os::fd::OwnedFd, thread};

use soma::{BackendFailureKind, OutputChannel};
use soma_guest::GuestCommand;
use soma_jail::ControlSocket;
use soma_vmm::control::{Reply, Request};
use soma_vmm::{OperationId, control::OutputStream};

use super::Jailed;

fn command() -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), Vec::new(), 1_000, 64).expect("command")
}

/// A worker end that reads one request and then sends `replies`, one packet each.
fn worker(end: OwnedFd, replies: Vec<String>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let socket = ControlSocket::adopt(end);
        let request = socket
            .receive(
                soma_vmm::control::MAX_REQUEST_BYTES,
                std::time::Duration::from_secs(5),
            )
            .expect("a request");
        for reply in replies {
            socket.send(&reply).expect("send");
        }
        request
    })
}

fn executed(stdout: u64, stderr: u64) -> String {
    // The operation and instance are not compared by the supervisor's streaming reader, which
    // reads status and counts; they only have to parse.
    format!(
        "executed operation={} instance={} status=code:0 stdout={stdout} stderr={stderr}",
        "01".repeat(16),
        "02".repeat(16)
    )
}

fn chunk(stream: OutputStream, bytes: &[u8]) -> String {
    Reply::Chunk(stream, bytes).encode()
}

#[test]
fn chunks_reach_the_sink_and_the_totals_match_the_final_reply() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    let worker = worker(
        far,
        vec![
            chunk(OutputStream::Stdout, b"ab"),
            chunk(OutputStream::Stderr, b"w"),
            chunk(OutputStream::Stdout, b"c"),
            executed(3, 1),
        ],
    );
    let mut seen = Vec::new();
    let completed = jailed
        .execute_streaming(&command(), &mut |channel, bytes| {
            seen.push((channel, bytes.to_vec()));
        })
        .expect("completes");
    assert_eq!(completed.stdout, b"abc");
    assert_eq!(completed.stderr, b"w");
    assert_eq!(
        seen,
        vec![
            (OutputChannel::Stdout, b"ab".to_vec()),
            (OutputChannel::Stderr, b"w".to_vec()),
            (OutputChannel::Stdout, b"c".to_vec()),
        ]
    );
    let request = worker.join().expect("worker");
    assert!(request.starts_with("execute_stream "));
    assert!(!jailed.poisoned);
}

#[test]
fn counts_that_disagree_with_the_chunks_poison_the_handle() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    let _worker = worker(
        far,
        vec![chunk(OutputStream::Stdout, b"ab"), executed(3, 0)],
    );
    let failure = jailed.execute_streaming(&command(), &mut |_, _| {}).err();
    assert_eq!(failure, Some(BackendFailureKind::GuestFailure));
    assert!(jailed.poisoned);
}

#[test]
fn output_past_the_allowance_poisons_the_handle_and_is_not_delivered() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    let command = GuestCommand::new(b"/bin/true".to_vec(), Vec::new(), 1_000, 4).expect("command");
    let _worker = worker(
        far,
        vec![
            chunk(OutputStream::Stdout, b"abc"),
            chunk(OutputStream::Stdout, b"de"),
        ],
    );
    let mut delivered = 0;
    let failure = jailed
        .execute_streaming(&command, &mut |_, bytes| delivered += bytes.len())
        .err();
    assert_eq!(failure, Some(BackendFailureKind::GuestFailure));
    assert_eq!(delivered, 3);
    assert!(jailed.poisoned);
}

#[test]
fn a_worker_that_predates_the_request_refuses_it_and_the_handle_stays_usable() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    let _worker = worker(far, vec!["rejected unknown request".to_owned()]);
    let failure = jailed
        .execute_streaming(&command(), &mut |_, _| panic!("no chunk"))
        .err();
    assert_eq!(failure, Some(BackendFailureKind::WorkloadRejected));
    assert!(
        !jailed.poisoned,
        "a stated refusal is not an uncertain exchange"
    );
}

#[test]
fn a_worker_that_goes_silent_poisons_the_handle() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    // The worker reads the request and hangs up without answering.
    let worker = worker(far, Vec::new());
    let failure = jailed.execute_streaming(&command(), &mut |_, _| {}).err();
    let _ = worker.join();
    assert_eq!(failure, Some(BackendFailureKind::Unavailable));
    assert!(jailed.poisoned);
}

#[test]
fn a_poisoned_handle_refuses_without_addressing_the_worker() {
    let (supervisor, _far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    jailed.poisoned = true;
    let failure = jailed.execute_streaming(&command(), &mut |_, _| {}).err();
    assert_eq!(failure, Some(BackendFailureKind::Unavailable));
}

#[test]
fn the_request_the_supervisor_sends_decodes_as_a_streaming_request() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    let worker = worker(far, vec![executed(0, 0)]);
    jailed
        .execute_streaming(&command(), &mut |_, _| {})
        .expect("completes");
    let packet = worker.join().expect("worker");
    assert!(matches!(
        Request::decode(&packet),
        Ok(Request::ExecuteStreaming(_))
    ));
    let _ = OperationId::new([1; 16]);
}

fn command_with_stdin(bytes: usize) -> GuestCommand {
    GuestCommand::new(b"/bin/cat".to_vec(), Vec::new(), 1_000, 64)
        .and_then(|command| {
            command.with_context(soma_guest::CommandContext {
                stdin: vec![b'x'; bytes],
                ..soma_guest::CommandContext::default()
            })
        })
        .expect("a command the guest protocol itself admits")
}

/// Standard input travels hex encoded inside one control packet, so the packet, not the guest
/// protocol, is what bounds it on the jailed road. A request the packet cannot carry is refused
/// by name before any worker is addressed, instead of being cut short by the kernel or hanging.
#[test]
fn stdin_the_control_packet_cannot_carry_is_refused_before_a_worker_is_addressed() {
    for stdin in [8 * 1024, 16 * 1024, 32 * 1024] {
        for streaming in [false, true] {
            let (supervisor, far) = ControlSocket::pair().expect("pair");
            let mut jailed = Jailed::for_test(supervisor);
            let started = std::time::Instant::now();
            let failure = if streaming {
                jailed
                    .execute_streaming(&command_with_stdin(stdin), &mut |_, _| panic!("no chunk"))
                    .err()
            } else {
                jailed.execute(&command_with_stdin(stdin)).err()
            };
            assert_eq!(
                failure,
                Some(BackendFailureKind::WorkloadRejected),
                "{stdin} {streaming}"
            );
            assert!(
                started.elapsed() < std::time::Duration::from_secs(1),
                "the refusal must not wait"
            );
            assert!(
                !jailed.poisoned,
                "a refusal made before the wire leaves the handle usable"
            );
            // Nothing was sent: the worker end sees the supervisor close without a request.
            drop(jailed);
            let socket = ControlSocket::adopt(far);
            assert!(
                socket
                    .receive(
                        soma_vmm::control::MAX_REQUEST_BYTES,
                        std::time::Duration::from_millis(200)
                    )
                    .is_err(),
                "a worker was addressed with a request the packet cannot carry"
            );
        }
    }
}

#[test]
fn stdin_that_fits_the_packet_is_sent_whole() {
    let (supervisor, far) = ControlSocket::pair().expect("pair");
    let mut jailed = Jailed::for_test(supervisor);
    let worker = worker(far, vec![executed(0, 0)]);
    let _ = jailed.execute_streaming(&command_with_stdin(7 * 1024), &mut |_, _| {});
    let request = worker.join().expect("worker");
    assert!(request.starts_with("execute_stream "), "{}", &request[..24]);
    assert!(
        request.len() > 14 * 1024,
        "the whole standard input travelled hex encoded"
    );
}
