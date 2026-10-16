//! What the transport does when there is no machine behind it.
//!
//! Every case here is about the seam rather than the sandbox: a name with nothing serving it, a
//! socket something already answers on, a line that will not fit, and a refusal that has to mean
//! the same thing on both sides of a process boundary. None of it needs KVM, which is the point:
//! these are the paths a caller reaches when the machine is exactly what is missing.

use std::{io::BufReader, os::unix::net::UnixListener};

use soma::{BackendFailureKind, GenerationId, InstanceId, MachineShape, OperationId};

use super::{
    channel,
    wire::{Call, LaunchWire, Refusal},
};

fn instance(value: &str) -> InstanceId {
    InstanceId::new(value).expect("a 32 character lowercase hexadecimal identity")
}

const ONE: &str = "0123456789abcdef0123456789abcdef";
const TWO: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn every_refusal_means_the_same_kind_on_both_sides() {
    let kinds = [
        BackendFailureKind::Unsupported,
        BackendFailureKind::Unavailable,
        BackendFailureKind::ResourceConflict,
        BackendFailureKind::WorkloadRejected,
        BackendFailureKind::IsolationFailure,
        BackendFailureKind::GuestFailure,
        BackendFailureKind::Timeout,
        BackendFailureKind::OutputLimit,
        BackendFailureKind::CleanupFailure,
    ];
    for kind in kinds {
        let encoded = serde_json::to_vec(&Refusal::from(kind)).expect("a refusal encodes");
        let decoded: Refusal = serde_json::from_slice(&encoded).expect("a refusal decodes");
        assert_eq!(
            BackendFailureKind::from(decoded),
            kind,
            "a refusal must not change kind on the way across"
        );
    }
}

#[test]
fn an_instance_with_no_host_is_absent_rather_than_broken() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    assert!(
        channel::connect(directory.path(), &instance(ONE)).is_err(),
        "nothing serves this Instance, so no connection may be reported"
    );
}

#[test]
fn a_socket_nothing_answers_on_is_removed_by_the_lookup_that_finds_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = channel::socket_path(directory.path(), &instance(ONE));
    // A host that was killed leaves its socket behind; the next lookup must not report it as one.
    drop(UnixListener::bind(&socket).expect("a socket to abandon"));
    assert!(socket.exists(), "the abandoned socket exists to begin with");

    assert!(channel::connect(directory.path(), &instance(ONE)).is_err());
    assert!(
        !socket.exists(),
        "a socket no process answers on must not survive the lookup that found it"
    );
}

#[test]
fn binding_refuses_a_socket_something_already_answers_on() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = channel::socket_path(directory.path(), &instance(TWO));
    let held = channel::bind(&socket).expect("the first host binds");

    assert!(
        channel::bind(&socket).is_err(),
        "a second host must not take over an Instance another is serving"
    );
    let _probe = held
        .accept()
        .expect("the first host drains the liveness probe");
    drop(held);

    // Once the first host is gone the name is free again, which is what makes a killed host
    // recoverable rather than a permanently poisoned identity.
    assert!(channel::bind(&socket).is_ok());
}

#[test]
fn one_line_carries_one_call() {
    let mut written = Vec::new();
    let call = Call::Cleanup {
        instance_id: instance(ONE),
        forced: true,
    };
    channel::write_line(&mut written, &call).expect("a call encodes");
    let (terminator, body) = written.split_last().expect("an encoded call is not empty");
    assert_eq!(*terminator, b'\n', "a call ends at a newline");
    assert!(
        !body.contains(&b'\n'),
        "a call is exactly one line, because the reader stops at the first newline"
    );

    let mut reader = BufReader::new(written.as_slice());
    let decoded = channel::read_line::<Call>(&mut reader).expect("a call decodes");
    assert!(matches!(
        decoded,
        Call::Cleanup { forced: true, .. } // the release method has to survive the crossing
    ));
}

#[test]
fn a_line_that_is_not_a_call_is_refused_rather_than_guessed_at() {
    let mut reader = BufReader::new(&b"{\"execute\":{}}\n"[..]);
    assert!(
        channel::read_line::<Call>(&mut reader).is_none(),
        "a malformed line must not become some other call"
    );

    let mut empty = BufReader::new(&b""[..]);
    assert!(
        channel::read_line::<Call>(&mut empty).is_none(),
        "a peer that is gone says nothing rather than something"
    );
}

/// A directory one byte too deep to name a socket in cannot hold a host, and the whole point of
/// asking is that `bind` reports that as an ordinary failure to start. This pins the boundary at
/// the byte, because a check that is nearly right refuses launches that would have worked.
#[test]
fn a_directory_too_deep_to_name_a_socket_in_is_not_addressable() {
    let name = InstanceId::LENGTH + ".sock".len() + 1 + 1;
    let longest = std::path::PathBuf::from("/".repeat(108 - name));
    let one_too_long = std::path::PathBuf::from("/".repeat(108 - name + 1));

    assert!(channel::addressable(&longest));
    assert!(!channel::addressable(&one_too_long));
    assert!(
        channel::socket_path(&longest, &instance(ONE))
            .as_os_str()
            .len()
            < 108
    );
}

fn launch_wire(skip_warm: bool) -> LaunchWire {
    LaunchWire {
        volume: None,
        socket: "/run/soma/one.sock".into(),
        operation_id: OperationId::new(ONE).expect("operation"),
        instance_id: instance(TWO),
        reference: "base".to_owned(),
        generation_id: GenerationId::new(format!("sha256:{}", "3".repeat(64)))
            .expect("canonical digest"),
        manifest: Vec::new(),
        shape: MachineShape::new(1, 512, 0).expect("shape"),
        skip_warm,
    }
}

#[test]
fn a_launch_that_warms_is_written_exactly_as_before_the_skip_existed() {
    let written = serde_json::to_value(launch_wire(false)).expect("encodes");

    assert!(written.get("skip_warm").is_none(), "{written}");
}

#[test]
fn a_skipped_warm_crosses_the_launch_channel() {
    let written = serde_json::to_string(&launch_wire(true)).expect("encodes");
    let read: LaunchWire = serde_json::from_str(&written).expect("decodes");

    assert!(read.skip_warm);
}

#[test]
fn a_launch_from_a_parent_that_predates_the_skip_still_warms() {
    let mut written = serde_json::to_value(launch_wire(false)).expect("encodes");
    written
        .as_object_mut()
        .expect("an object")
        .remove("skip_warm");
    let read: LaunchWire = serde_json::from_value(written).expect("decodes");

    assert!(!read.skip_warm);
}

mod streaming {
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
            wire::{Answer, Call, Channel, Refusal},
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
            host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {})
                .err();
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
            host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {})
                .err();
        assert!(matches!(
            failure,
            Some(HostFailure::Refused(BackendFailureKind::Unavailable))
        ));
    }

    #[test]
    fn nothing_listening_is_absence_not_a_broken_exchange() {
        let directory = tempfile::tempdir().expect("directory");
        let failure =
            host::execute_streaming(directory.path(), &instance(ONE), parts(64), &mut |_, _| {})
                .err();
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
}
