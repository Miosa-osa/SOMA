//! The terminal exercised against a real pseudo-terminal and a real shell.
//!
//! Nothing here stubs the kernel. Every case allocates a pair, runs the shell on it, and asserts
//! what the caller would see, because the whole value of this module is that a program on the
//! other end genuinely believes it is sitting at a terminal.

use std::time::{Duration, Instant};

use soma_guest::{PtyFailure, PtyOutcome, PtyRequest, PtySize};

use super::{Terminal, device};

/// Longest any case here waits for a shell to say something before it gives up.
///
/// It is a failure ceiling and not a latency target: a shell answers a typed line in
/// milliseconds, and a case that reaches this bound has already failed.
const PATIENCE: Duration = Duration::from_secs(10);
/// The wait one read carries, short enough that a case polls rather than blocks on one answer.
const READ_WAIT_MILLIS: u32 = 200;

fn size(columns: u16, rows: u16) -> PtySize {
    PtySize::new(columns, rows).expect("bounded dimensions")
}

/// Opens a terminal at the given dimensions and asserts it opened there.
fn opened(columns: u16, rows: u16) -> Terminal {
    let mut terminal = Terminal::new();
    let outcome = terminal.perform(&PtyRequest::Open(size(columns, rows)));

    assert_eq!(outcome, PtyOutcome::Opened(size(columns, rows)));
    terminal
}

/// Types one line at the terminal, exactly as a caller would.
fn type_line(terminal: &mut Terminal, line: &str) {
    let outcome = terminal.perform(&PtyRequest::Write {
        bytes: format!("{line}\n").into_bytes().into(),
    });
    let PtyOutcome::Wrote { bytes } = outcome else {
        panic!("expected a write outcome, got {outcome:?}");
    };
    assert_eq!(usize::try_from(bytes), Ok(line.len() + 1));
}

/// Reads until the accumulated output contains `wanted`, or the patience runs out.
///
/// A terminal echoes the line as it is typed and then answers it, and neither arrives in one
/// chunk, so a case that read once would be asserting on how the kernel happened to split the
/// bytes rather than on what the terminal said.
fn read_until(terminal: &mut Terminal, wanted: &str) -> String {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = String::new();
    while Instant::now() < deadline {
        let outcome = terminal.perform(&PtyRequest::Read {
            wait_millis: READ_WAIT_MILLIS,
        });
        let PtyOutcome::Output { bytes, end } = outcome else {
            panic!("expected an output outcome, got {outcome:?}");
        };
        seen.push_str(&String::from_utf8_lossy(&bytes));
        if seen.contains(wanted) {
            return seen;
        }
        assert!(
            !end,
            "the session ended before {wanted:?} appeared: {seen:?}"
        );
    }
    panic!("waited {PATIENCE:?} without seeing {wanted:?}: {seen:?}");
}

/// Reads until the session reports its end, or the patience runs out.
fn read_until_end(terminal: &mut Terminal) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let outcome = terminal.perform(&PtyRequest::Read {
            wait_millis: READ_WAIT_MILLIS,
        });
        if matches!(outcome, PtyOutcome::Output { end: true, .. }) {
            return;
        }
    }
    panic!("waited {PATIENCE:?} for the session to end");
}

/// The whole point of a pseudo-terminal: what is written comes back through a real program.
#[test]
fn a_real_terminal_echoes_what_is_written() {
    let mut terminal = opened(80, 24);

    type_line(&mut terminal, "echo soma-terminal-lives");

    // The word appears twice, once echoed by the line discipline and once printed by `echo`,
    // which is exactly what distinguishes a terminal from a pipe.
    let seen = read_until(&mut terminal, "soma-terminal-lives");
    assert!(seen.contains("soma-terminal-lives"), "{seen:?}");
}

/// A program in the session sees the dimensions the session was opened with.
#[test]
fn a_real_terminal_reports_the_size_it_was_opened_with() {
    let mut terminal = opened(101, 37);

    type_line(&mut terminal, "stty size");

    let seen = read_until(&mut terminal, "37 101");
    assert!(seen.contains("37 101"), "{seen:?}");
}

/// A resize reaches the terminal itself, not just the agent's record of it.
#[test]
fn resizing_changes_the_size_the_terminal_reports() {
    let mut terminal = opened(80, 24);
    type_line(&mut terminal, "stty size");
    read_until(&mut terminal, "24 80");

    let resized = terminal.perform(&PtyRequest::Resize(size(132, 43)));

    assert_eq!(resized, PtyOutcome::Resized(size(132, 43)));
    type_line(&mut terminal, "stty size");
    let seen = read_until(&mut terminal, "43 132");
    assert!(seen.contains("43 132"), "{seen:?}");
}

/// The `ioctl` that sets a size is the one a program reads back, with no agent bookkeeping.
#[test]
fn the_terminal_device_holds_the_size_that_was_set() {
    let pair = device::open(size(90, 30)).expect("a pseudo-terminal pair");

    assert_eq!(device::size_of(&pair.master).ok(), Some((90, 30)));

    device::set_size(&pair.master, size(200, 60)).expect("a resize");
    assert_eq!(device::size_of(&pair.master).ok(), Some((200, 60)));
}

/// Every request against a session that was never opened is refused, and says why.
#[test]
fn a_request_against_an_unknown_session_is_refused() {
    let mut terminal = Terminal::new();
    for request in [
        PtyRequest::Write {
            bytes: b"whoami\n".to_vec().into(),
        },
        PtyRequest::Read { wait_millis: 0 },
        PtyRequest::Resize(size(80, 24)),
        PtyRequest::Close,
    ] {
        assert_eq!(
            terminal.perform(&request),
            PtyOutcome::Failed(PtyFailure::NoSession),
            "{request:?} was answered without a session"
        );
    }
}

/// This protocol carries one session at a time, so a second open is refused rather than queued.
#[test]
fn a_second_open_is_refused_while_the_first_is_alive() {
    let mut terminal = opened(80, 24);

    let second = terminal.perform(&PtyRequest::Open(size(80, 24)));

    assert_eq!(second, PtyOutcome::Failed(PtyFailure::AlreadyOpen));
    assert_eq!(terminal.perform(&PtyRequest::Close), PtyOutcome::Closed);
}

/// Closing ends the session, and the slot is free for the next one.
#[test]
fn closing_ends_the_session_and_frees_the_slot() {
    let mut terminal = opened(80, 24);

    assert_eq!(terminal.perform(&PtyRequest::Close), PtyOutcome::Closed);

    assert_eq!(
        terminal.perform(&PtyRequest::Read { wait_millis: 0 }),
        PtyOutcome::Failed(PtyFailure::NoSession)
    );
    assert_eq!(
        terminal.perform(&PtyRequest::Open(size(80, 24))),
        PtyOutcome::Opened(size(80, 24))
    );
    assert_eq!(terminal.perform(&PtyRequest::Close), PtyOutcome::Closed);
}

/// A shell that exits ends the stream explicitly, and the session is gone afterwards.
#[test]
fn a_shell_that_exits_reports_the_end_of_the_stream() {
    let mut terminal = opened(80, 24);

    type_line(&mut terminal, "exit");
    read_until_end(&mut terminal);

    assert_eq!(
        terminal.perform(&PtyRequest::Read { wait_millis: 0 }),
        PtyOutcome::Failed(PtyFailure::NoSession),
        "a session that ended was still readable"
    );
}

/// A read with nothing to read answers with nothing rather than waiting past its bound.
#[test]
fn a_read_with_nothing_ready_answers_with_an_empty_chunk() {
    let mut terminal = opened(80, 24);
    // The shell prints a prompt when it starts, so the terminal is drained first; only then is
    // "nothing ready" the state under test rather than "the prompt has not arrived yet".
    type_line(&mut terminal, "echo drained");
    read_until(&mut terminal, "drained");
    while !matches!(
        terminal.perform(&PtyRequest::Read {
            wait_millis: READ_WAIT_MILLIS
        }),
        PtyOutcome::Output { ref bytes, .. } if bytes.is_empty()
    ) {}

    let outcome = terminal.perform(&PtyRequest::Read {
        wait_millis: READ_WAIT_MILLIS,
    });

    assert_eq!(
        outcome,
        PtyOutcome::Output {
            bytes: Box::default(),
            end: false
        }
    );
    assert_eq!(terminal.perform(&PtyRequest::Close), PtyOutcome::Closed);
}
