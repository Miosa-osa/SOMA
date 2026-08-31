//! Live proof that the terminal is a real pseudo-terminal and not a pipe with a size field.
//!
//! Four properties separate the two, and all four are asserted here against the shell the guest
//! agent starts. A terminal echoes what is typed at it, because the line discipline does that
//! and a pipe does not. It reports its own dimensions to a program that asks the kernel, which
//! is where an `ioctl` answer differs from a number the protocol merely carried. It changes
//! those dimensions when it is resized. And it reports the end of the session once the shell
//! that owned it has gone, which is the flag a caller drains a terminal by.

use std::time::Instant;

use soma_guest::{PtyFailure, PtyOutcome, PtyRequest, PtySize};
use soma_kvm::x86_64::SandboxMachine;

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_snapshot_restore_capability::assert_no_leak,
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_instance as instance,
    x86_64_snapshot_restore_workload::{Session, Workload},
};

/// The size the session opens at.
const OPEN_COLUMNS: u16 = 80;
const OPEN_ROWS: u16 = 24;
/// The size it is resized to, chosen so neither dimension could be mistaken for the first.
const NEXT_COLUMNS: u16 = 132;
const NEXT_ROWS: u16 = 43;
/// Longest one read waits for the first byte.
const WAIT_MILLIS: u32 = 4_000;
/// Longest one drain collects for before it gives up on the text it is waiting for.
const DRAIN_READS: usize = 24;

/// What the terminal proof retains.
pub struct Terminal {
    pub opened: PtyOutcome,
    pub after_open: String,
    pub resized: PtyOutcome,
    pub after_resize: String,
    /// Closing the session while its shell is still alive.
    pub closed_live: PtyOutcome,
    /// Writing to the slot the close emptied.
    pub after_close: PtyOutcome,
    /// Opening a second session once the first is gone.
    pub reopened: PtyOutcome,
    pub ended: bool,
    pub tail: String,
    /// Closing a session that already reported its own end.
    pub closed_after_end: PtyOutcome,
}

struct TerminalWorkload;

impl Workload for TerminalWorkload {
    type Output = Terminal;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Terminal), String> {
        let open = PtySize::new(OPEN_COLUMNS, OPEN_ROWS).map_err(|error| format!("{error}"))?;
        let (session, opened) = ask(session, PtyRequest::Open(open))?;
        let (session, _) = drain(session, "")?;
        let (session, _) = type_line(session, b"stty size\n")?;
        let (session, after_open) = drain(session, &format!("{OPEN_ROWS} {OPEN_COLUMNS}"))?;

        let next = PtySize::new(NEXT_COLUMNS, NEXT_ROWS).map_err(|error| format!("{error}"))?;
        let (session, resized) = ask(session, PtyRequest::Resize(next))?;
        let (session, _) = type_line(session, b"stty size\n")?;
        let (session, after_resize) = drain(session, &format!("{NEXT_ROWS} {NEXT_COLUMNS}"))?;

        // Closing a live session is one answer; a session that ended on its own is another, and
        // the two are proved separately because only the first has anything left to close.
        let (session, closed_live) = ask(session, PtyRequest::Close)?;
        let (session, after_close) = ask(
            session,
            PtyRequest::Write {
                bytes: b"nobody is listening\n".as_slice().into(),
            },
        )?;

        let (session, reopened) = ask(session, PtyRequest::Open(open))?;
        let (session, _) = drain(session, "")?;
        let (session, _) = type_line(session, b"exit\n")?;
        let (session, tail, ended) = drain_to_end(session)?;
        // The slot is empty once the end is reported, so this asks the guest to close a session
        // it no longer has, and the refusal is what says the end really ended it.
        let (session, closed_after_end) = ask(session, PtyRequest::Close)?;
        Ok((
            session,
            Terminal {
                opened,
                after_open,
                resized,
                after_resize,
                closed_live,
                after_close,
                reopened,
                ended,
                tail,
                closed_after_end,
            },
        ))
    }
}

/// Issues one terminal request and keeps its answer.
fn ask(session: Session<'_>, request: PtyRequest) -> Result<(Session<'_>, PtyOutcome), String> {
    session
        .pty(request)
        .map_err(|error| format!("terminal request: {error}"))
}

/// Types one line at the terminal, requiring that every byte of it was taken.
fn type_line<'a>(session: Session<'a>, line: &[u8]) -> Result<(Session<'a>, PtyOutcome), String> {
    let (session, outcome) = ask(session, PtyRequest::Write { bytes: line.into() })?;
    match outcome {
        PtyOutcome::Wrote { bytes } if usize::try_from(bytes) == Ok(line.len()) => {
            Ok((session, outcome))
        }
        other => Err(format!("typing {} bytes answered {other:?}", line.len())),
    }
}

/// Reads until `wanted` has been seen, or until the reads run out.
fn drain<'a>(session: Session<'a>, wanted: &str) -> Result<(Session<'a>, String), String> {
    let mut session = session;
    let mut collected = String::new();
    let started = Instant::now();
    for _ in 0..DRAIN_READS {
        let (next, outcome) = ask(
            session,
            PtyRequest::Read {
                wait_millis: WAIT_MILLIS,
            },
        )?;
        session = next;
        let PtyOutcome::Output { bytes, end } = outcome else {
            return Err(format!("a read answered {outcome:?}"));
        };
        collected.push_str(&String::from_utf8_lossy(&bytes));
        if end || (!wanted.is_empty() && collected.contains(wanted)) {
            break;
        }
        if wanted.is_empty() && bytes.is_empty() {
            break;
        }
    }
    eprintln!(
        "[pty] drained {} bytes in {:?} looking for {wanted:?}",
        collected.len(),
        started.elapsed()
    );
    Ok((session, collected))
}

/// Reads until the session reports its end, or until the reads run out.
fn drain_to_end(session: Session<'_>) -> Result<(Session<'_>, String, bool), String> {
    let mut session = session;
    let mut collected = String::new();
    for _ in 0..DRAIN_READS {
        let (next, outcome) = ask(
            session,
            PtyRequest::Read {
                wait_millis: WAIT_MILLIS,
            },
        )?;
        session = next;
        let PtyOutcome::Output { bytes, end } = outcome else {
            return Err(format!("a read answered {outcome:?}"));
        };
        collected.push_str(&String::from_utf8_lossy(&bytes));
        if end {
            return Ok((session, collected, true));
        }
    }
    Ok((session, collected, false))
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_real_terminal_echoes_reports_its_size_resizes_and_reports_its_end() {
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "pty", 45, TerminalWorkload);
    assert_no_leak(&restored);

    let terminal = &restored.output;
    let open = PtySize::new(OPEN_COLUMNS, OPEN_ROWS).expect("a bounded size");
    let next = PtySize::new(NEXT_COLUMNS, NEXT_ROWS).expect("a bounded size");
    assert_eq!(terminal.opened, PtyOutcome::Opened(open));
    assert_eq!(terminal.resized, PtyOutcome::Resized(next));

    eprintln!("[pty] after open:\n{}", terminal.after_open);
    // The line discipline echoes what was typed, so the command itself appears in the output
    // before its result does. A pipe would carry the result and never the command.
    assert!(
        terminal.after_open.contains("stty size"),
        "the terminal did not echo what was typed at it: {:?}",
        terminal.after_open
    );
    assert!(
        terminal
            .after_open
            .contains(&format!("{OPEN_ROWS} {OPEN_COLUMNS}")),
        "the terminal did not report the size it opened at: {:?}",
        terminal.after_open
    );
    eprintln!("[pty] after resize:\n{}", terminal.after_resize);
    assert!(
        terminal
            .after_resize
            .contains(&format!("{NEXT_ROWS} {NEXT_COLUMNS}")),
        "the terminal did not report its new size: {:?}",
        terminal.after_resize
    );

    assert_eq!(
        terminal.closed_live,
        PtyOutcome::Closed,
        "a live terminal refused to close"
    );
    assert!(
        matches!(
            terminal.after_close,
            PtyOutcome::Failed(PtyFailure::NoSession)
        ),
        "a closed terminal accepted a write: {:?}",
        terminal.after_close
    );
    assert_eq!(
        terminal.reopened,
        PtyOutcome::Opened(open),
        "the one terminal slot did not take a second session after the first closed"
    );
    assert!(
        terminal.ended,
        "the terminal never reported its end after its shell exited; tail={:?}",
        terminal.tail
    );
    // A session that reported its end is already gone, so there is nothing left to close. That
    // refusal is the guest saying the end was the end, not that the close failed.
    assert!(
        matches!(
            terminal.closed_after_end,
            PtyOutcome::Failed(PtyFailure::NoSession)
        ),
        "a terminal that reported its end still had a session to close: {:?}",
        terminal.closed_after_end
    );
}
