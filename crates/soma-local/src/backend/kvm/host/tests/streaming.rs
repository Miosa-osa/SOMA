//! The client side of a streaming command against a host that is only a socket.

use std::{
    io::BufReader,
    os::unix::net::UnixListener,
    sync::mpsc,
    thread::{self, JoinHandle},
};

use soma::{BackendFailureKind, CommandStatus, OutputChannel};

use super::{ONE, channel, instance};
use crate::backend::kvm::{
    evidence::{CommandParts, WireContext},
    host::{
        self, HostFailure,
        wire::exchange::Channel,
        wire::{Answer, Call, Refusal},
    },
};

fn parts(output: u64) -> CommandParts {
    CommandParts {
        program: b"/bin/true".to_vec(),
        arguments: Vec::new(),
        timeout_ms: 1_000,
        max_output_bytes: output,
        context: WireContext::default(),
    }
}

/// A host that reads one call, then writes `answers`, one line each, with a gate between the
/// first answer and the rest so a test can look at what the client saw in between.
fn serve(
    directory: &std::path::Path,
    answers: Vec<Answer>,
    gate: Option<mpsc::Receiver<()>>,
) -> JoinHandle<Call> {
    let id = instance(ONE);
    let listener = UnixListener::bind(channel::socket_path(directory, &id)).expect("bind");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("a client");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let call = channel::read_line::<Call>(&mut reader).expect("a call");
        for (index, answer) in answers.into_iter().enumerate() {
            if index == 1
                && let Some(gate) = &gate
            {
                let _ = gate.recv();
            }
            channel::write_line(&mut stream, &answer).expect("write");
        }
        call
    })
}

fn chunk(channel: Channel, bytes: &[u8]) -> Answer {
    Answer::Chunk {
        channel,
        bytes: bytes.to_vec(),
    }
}

#[test]
fn chunks_reach_the_sink_before_the_command_ends() {
    let directory = tempfile::tempdir().expect("directory");
    let (release, gate) = mpsc::channel();
    let host = serve(
        directory.path(),
        vec![
            chunk(Channel::Stdout, b"first"),
            chunk(Channel::Stderr, b"warn"),
            Answer::StreamEnded {
                status: CommandStatus::Exited { code: 3 },
                stdout_bytes: 5,
                stderr_bytes: 4,
            },
        ],
        Some(gate),
    );
    let (seen, received) = mpsc::channel();
    let id = instance(ONE);
    let client = thread::scope(|scope| {
        let handle = scope.spawn(|| {
            host::execute_streaming(directory.path(), &id, parts(64), &mut |channel, bytes| {
                seen.send((channel, bytes.to_vec())).expect("listening");
            })
        });
        // The host is holding everything after the first line, so a chunk that arrives now
        // arrived while the command was still running.
        assert_eq!(
            received.recv_timeout(std::time::Duration::from_secs(5)),
            Ok((OutputChannel::Stdout, b"first".to_vec()))
        );
        release.send(()).expect("release");
        handle.join().expect("client thread")
    });
    let executed = client.ok().expect("completes");
    assert_eq!(executed.stdout, b"first");
    assert_eq!(executed.stderr, b"warn");
    assert!(matches!(executed.status, CommandStatus::Exited { code: 3 }));
    assert!(matches!(
        host.join().expect("host"),
        Call::ExecuteStreaming { .. }
    ));
}

#[test]
fn a_host_that_reports_other_counts_than_it_sent_is_not_trusted() {
    let directory = tempfile::tempdir().expect("directory");
    let _host = serve(
        directory.path(),
        vec![
            chunk(Channel::Stdout, b"abc"),
            Answer::StreamEnded {
                status: CommandStatus::Exited { code: 0 },
                stdout_bytes: 4,
                stderr_bytes: 0,
            },
        ],
        None,
    );
    let failure =
        host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {}).err();
    assert!(matches!(
        failure,
        Some(HostFailure::Refused(BackendFailureKind::GuestFailure))
    ));
}

#[test]
fn chunks_past_the_output_allowance_are_refused() {
    let directory = tempfile::tempdir().expect("directory");
    let _host = serve(
        directory.path(),
        vec![
            chunk(Channel::Stdout, b"abc"),
            chunk(Channel::Stdout, b"defg"),
        ],
        None,
    );
    let mut delivered = 0;
    let failure = host::execute_streaming(
        directory.path(),
        &instance(ONE),
        parts(5),
        &mut |_, bytes| delivered += bytes.len(),
    )
    .err();
    assert!(matches!(
        failure,
        Some(HostFailure::Refused(BackendFailureKind::GuestFailure))
    ));
    assert_eq!(
        delivered, 3,
        "the chunk that broke the allowance was not delivered"
    );
}

#[test]
fn a_refusal_before_any_chunk_is_the_refusal() {
    let directory = tempfile::tempdir().expect("directory");
    let _host = serve(
        directory.path(),
        vec![Answer::Refused(Refusal::from(
            BackendFailureKind::WorkloadRejected,
        ))],
        None,
    );
    let failure =
        host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {
            panic!("no chunk was sent")
        })
        .err();
    assert!(matches!(
        failure,
        Some(HostFailure::Refused(BackendFailureKind::WorkloadRejected))
    ));
}

#[test]
fn a_host_that_hangs_up_mid_stream_is_unavailable() {
    let directory = tempfile::tempdir().expect("directory");
    let _host = serve(directory.path(), vec![chunk(Channel::Stdout, b"abc")], None);
    let failure =
        host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {}).err();
    assert!(matches!(
        failure,
        Some(HostFailure::Refused(BackendFailureKind::Unavailable))
    ));
}

#[test]
fn nothing_listening_is_absence_not_a_broken_exchange() {
    let directory = tempfile::tempdir().expect("directory");
    let failure =
        host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {}).err();
    assert!(matches!(failure, Some(HostFailure::Absent)));
}

#[test]
fn the_streaming_call_is_its_own_variant() {
    let call = Call::ExecuteStreaming {
        instance_id: instance(ONE),
        program: b"/bin/true".to_vec(),
        arguments: Vec::new(),
        timeout_ms: 1,
        max_output_bytes: 1,
        context: WireContext::default(),
    };
    let line = serde_json::to_string(&call).expect("encodes");
    assert!(line.contains("execute_streaming"));
    assert!(!line.contains("\"execute\""));
}
