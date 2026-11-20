//! An argument past what a level 2 guest decodes, sent only to a guest that answered level 3.

use std::thread;

use crate::{
    ControlFailureClass, FileOutcome, GUEST_API_PROBE_PREFIX, GuestApi, GuestCommand, GuestControl,
    GuestRequest, MAX_ARGUMENT_BYTES, OperationId, TerminalStatus,
};

use super::local_guest::{control_error, repaired_owners};
use super::support::{MemoryIo, deadline};

/// A 12 KB shell script as one argument of `sh -lc`, the shape a real client sends.
fn heredoc() -> GuestCommand {
    GuestCommand::new(
        b"/bin/sh".to_vec(),
        vec![b"-lc".to_vec(), vec![b'#'; 12 * 1024]],
        10_000,
        64,
    )
    .expect("a command within the bounds")
}

fn operation(value: u8) -> OperationId {
    OperationId::new([value; 16]).expect("operation")
}

/// Answers the host's level probe as an agent at `level` does, and returns the guest idle again.
fn answer_probe(guest: GuestControl<MemoryIo>, level: u8) -> GuestControl<MemoryIo> {
    let (guest, request) = guest.next_request(deadline()).expect("probe request");
    let GuestRequest::File { request, .. } = request else {
        panic!("the probe is a file read");
    };
    let mut answer = format!("{level} ").into_bytes();
    answer.extend_from_slice(
        request
            .path()
            .strip_prefix(GUEST_API_PROBE_PREFIX)
            .expect("the probe path"),
    );
    answer.push(b'\n');
    guest
        .file_outcome(
            &FileOutcome::Read {
                bytes: answer.into(),
                end: true,
            },
            deadline(),
        )
        .expect("probe answer")
}

#[test]
fn a_long_argument_reaches_a_level_three_guest_byte_for_byte() {
    assert!(heredoc().needs_long_arguments());
    let at_bound = GuestCommand::new(
        b"/bin/sh".to_vec(),
        vec![b"-lc".to_vec(), vec![b'#'; MAX_ARGUMENT_BYTES]],
        10_000,
        64,
    )
    .expect("an argument at the bound");
    for sent in [heredoc(), at_bound] {
        let (host, guest, host_observed, guest_observed) = repaired_owners();
        let expected = sent.clone();
        let host_thread = thread::spawn(move || {
            let mut cache = None;
            let (host, api) = host
                .ensure_guest_api(&mut cache, || b"abc123".to_vec())
                .expect("probe");
            assert_eq!(api, GuestApi::Level(3));
            assert!(api.has_long_arguments());
            host.execute(operation(7), sent).expect("execute")
        });
        let guest = answer_probe(guest, 3);
        let (guest, request) = guest.next_request(deadline()).expect("execute");
        let GuestRequest::Execute { command, .. } = request else {
            panic!("the guest receives the command");
        };
        assert_eq!(command, expected);
        let _guest = guest
            .terminal(TerminalStatus::Exited(0), deadline())
            .expect("terminal");
        let (_host, outcome) = host_thread.join().expect("host thread");
        assert_eq!(outcome.status(), TerminalStatus::Exited(0));
        assert_eq!(host_observed.poison(), 0);
        assert_eq!(guest_observed.poison(), 0);
    }
}

#[test]
fn a_host_that_has_not_learned_level_three_sends_no_long_argument() {
    // A level 2 answer, and no probe at all: both leave the host without the knowledge that the
    // guest decodes the argument, so the command is failed before anything is sent.
    for probed in [Some(2), None] {
        let (host, guest, host_observed, _) = repaired_owners();
        let host_thread = thread::spawn(move || {
            let host = match probed {
                Some(_) => {
                    let mut cache = None;
                    host.ensure_guest_api(&mut cache, || b"abc123".to_vec())
                        .expect("probe")
                        .0
                }
                None => host,
            };
            host.execute(operation(7), heredoc())
                .map(|(_, outcome)| outcome)
        });
        if let Some(level) = probed {
            drop(answer_probe(guest, level));
        }
        let error = control_error(host_thread.join().expect("host thread"));
        assert_eq!(error.class(), ControlFailureClass::Lifecycle);
        assert_eq!(host_observed.poison(), 1);
    }
}

#[test]
fn a_level_two_guest_is_refused_a_long_argument_and_still_answers_the_next_command() {
    // The failure this gate exists for: a 12 KB argument to a guest whose agent decodes at most
    // 4096 bytes would end its session. Asked first, the host refuses it with nothing sent.
    let (host, guest, host_observed, guest_observed) = repaired_owners();
    let host_thread = thread::spawn(move || {
        let mut cache = None;
        let (host, admitted) = host
            .admits(&heredoc(), &mut cache, || b"abc123".to_vec())
            .expect("probe");
        assert!(!admitted, "a level 2 guest is not sent a long argument");
        // Only the first long command probes; the session remembers the answer.
        let (host, again) = host
            .admits(&heredoc(), &mut cache, || panic!("no second probe"))
            .expect("cached level");
        assert!(!again);
        let short = GuestCommand::new(b"/bin/true".to_vec(), vec![], 10_000, 64).expect("short");
        let (host, admitted) = host
            .admits(&short, &mut cache, || {
                panic!("no probe for a short command")
            })
            .expect("no probe");
        assert!(admitted);
        host.execute(operation(8), short).expect("the next command")
    });
    let guest = answer_probe(guest, 2);
    let (guest, request) = guest.next_request(deadline()).expect("the next request");
    let GuestRequest::Execute { operation, command } = request else {
        panic!("the next record the guest sees is the short command, never the refused one");
    };
    assert_eq!(operation, self::operation(8));
    assert!(!command.needs_long_arguments());
    let _guest = guest
        .terminal(TerminalStatus::Exited(0), deadline())
        .expect("terminal");

    let (_host, outcome) = host_thread.join().expect("host thread");
    assert_eq!(outcome.status(), TerminalStatus::Exited(0));
    assert_eq!(host_observed.poison(), 0);
    assert_eq!(guest_observed.poison(), 0);
}

#[test]
fn a_level_three_guest_admits_a_long_argument_after_one_probe() {
    let (host, guest, _, _) = repaired_owners();
    let host_thread = thread::spawn(move || {
        let mut cache = None;
        let (host, admitted) = host
            .admits(&heredoc(), &mut cache, || b"abc123".to_vec())
            .expect("probe");
        assert!(admitted);
        assert_eq!(cache, Some(GuestApi::Level(3)));
        host.execute(operation(7), heredoc()).expect("execute")
    });
    let guest = answer_probe(guest, 3);
    let (guest, request) = guest.next_request(deadline()).expect("execute");
    assert!(matches!(request, GuestRequest::Execute { .. }));
    let _guest = guest
        .terminal(TerminalStatus::Exited(0), deadline())
        .expect("terminal");
    let (_host, outcome) = host_thread.join().expect("host thread");
    assert_eq!(outcome.status(), TerminalStatus::Exited(0));
}
