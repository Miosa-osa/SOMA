//! Output reaches a streaming caller while the command is still running, and a streaming
//! caller observes exactly the command a buffering caller does.

use std::{sync::mpsc, thread, time::Duration};

use crate::{
    ControlFailureClass, GuestCommand, GuestMessage, HostMessage, OutputChunk, OutputSource,
    TerminalReport, TerminalStatus,
};

use super::host_file::{control_error, operation, repaired_host};

fn command(output: u64) -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), vec![], 10, output).expect("command")
}

fn chunk(bytes: &[u8]) -> OutputChunk {
    OutputChunk::new(bytes.to_vec()).expect("chunk")
}

fn report(status: TerminalStatus, stdout: u32, stderr: u32) -> TerminalReport {
    TerminalReport::new(status, stdout, stderr).expect("report")
}

#[test]
fn a_chunk_reaches_the_callback_before_the_command_ends() {
    let (host, mut raw, _observed) = repaired_host();
    let execute = operation(7);
    let (seen, received) = mpsc::channel();
    let host_thread = thread::spawn(move || {
        host.execute_streaming(execute, command(64), move |source, bytes| {
            seen.send((source, bytes.to_vec()))
                .expect("test is listening");
        })
    });
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));

    raw.send(GuestMessage::stdout(execute, chunk(b"first")));
    // The terminal has not been sent, so the command is still running as far as the host knows.
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Ok((OutputSource::Stdout, b"first".to_vec()))
    );
    raw.send(GuestMessage::stderr(execute, chunk(b"warn")));
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Ok((OutputSource::Stderr, b"warn".to_vec()))
    );
    assert!(received.try_recv().is_err(), "nothing else arrived yet");

    raw.send(GuestMessage::stdout(execute, chunk(b"-second")));
    raw.send(GuestMessage::terminal(
        execute,
        report(TerminalStatus::Exited(3), 12, 4),
    ));
    let (_host, outcome) = host_thread.join().expect("host thread").expect("executes");
    assert_eq!(outcome.status(), TerminalStatus::Exited(3));
    assert_eq!(outcome.stdout(), b"first-second");
    assert_eq!(outcome.stderr(), b"warn");
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Ok((OutputSource::Stdout, b"-second".to_vec()))
    );
}

#[test]
fn a_streaming_caller_and_a_buffering_caller_observe_the_same_command() {
    let respond = |raw: &mut super::support::RawGuest, execute| {
        raw.send(GuestMessage::stdout(execute, chunk(b"ab")));
        raw.send(GuestMessage::stderr(execute, chunk(b"cd")));
        raw.send(GuestMessage::stdout(execute, chunk(b"ef")));
        raw.send(GuestMessage::terminal(
            execute,
            report(TerminalStatus::Exited(0), 4, 2),
        ));
    };

    let (host, mut raw, _) = repaired_host();
    let execute = operation(7);
    let buffered = thread::spawn(move || host.execute(execute, command(64)));
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    respond(&mut raw, execute);
    let (_, buffered) = buffered.join().expect("host thread").expect("executes");

    let (host, mut raw, _) = repaired_host();
    let streamed = thread::spawn(move || host.execute_streaming(execute, command(64), |_, _| {}));
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    respond(&mut raw, execute);
    let (_, streamed) = streamed.join().expect("host thread").expect("executes");

    assert_eq!(buffered.status(), streamed.status());
    assert_eq!(buffered.stdout(), streamed.stdout());
    assert_eq!(buffered.stderr(), streamed.stderr());
}

#[test]
fn a_chunk_past_the_allowance_never_reaches_the_callback() {
    let (host, mut raw, observed) = repaired_host();
    let execute = operation(7);
    let (seen, received) = mpsc::channel();
    let host_thread = thread::spawn(move || {
        host.execute_streaming(execute, command(5), move |source, bytes| {
            seen.send((source, bytes.to_vec()))
                .expect("test is listening");
        })
    });
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    raw.send(GuestMessage::stdout(execute, chunk(b"abc")));
    raw.send(GuestMessage::stdout(execute, chunk(b"defg")));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.class(), ControlFailureClass::Accounting);
    assert_eq!(observed.poison(), 1);
    assert_eq!(
        received.try_iter().collect::<Vec<_>>(),
        vec![(OutputSource::Stdout, b"abc".to_vec())],
        "the in-allowance chunk was delivered, the one that broke the allowance was not"
    );
}

#[test]
fn a_terminal_that_disagrees_with_the_streamed_chunks_still_poisons_the_session() {
    let (host, mut raw, observed) = repaired_host();
    let execute = operation(7);
    let host_thread =
        thread::spawn(move || host.execute_streaming(execute, command(64), |_, _| {}));
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    raw.send(GuestMessage::stdout(execute, chunk(b"abc")));
    raw.send(GuestMessage::terminal(
        execute,
        report(TerminalStatus::Exited(0), 2, 0),
    ));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.class(), ControlFailureClass::Accounting);
    assert_eq!(observed.poison(), 1);
}
