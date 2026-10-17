//! A streaming command reports each output chunk before it ends, and an uncertain stream ends
//! the session the way an uncertain buffered command does.

use std::sync::mpsc::channel;
use std::thread::JoinHandle;
use std::time::Duration;

use soma_guest::{GuestCommand, OutputSource, TerminalStatus};

use super::session::{Completed, Request, Response};
use super::{Session, SessionError};

fn command() -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), Vec::new(), 1_000, 1 << 20).expect("command")
}

fn completed() -> Completed {
    Completed {
        status: TerminalStatus::Exited(0),
        stdout: b"ab".to_vec(),
        stderr: Vec::new(),
    }
}

/// A session whose machine thread plays `script` once it receives a streaming command.
fn session_playing(
    script: impl FnOnce(&std::sync::mpsc::Sender<Response>) + Send + 'static,
) -> (Session, JoinHandle<Vec<&'static str>>) {
    let (request_tx, request_rx) = channel::<Request>();
    let (response_tx, response_rx) = channel();
    let stand_in = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut script = Some(script);
        while let Ok(request) = request_rx.recv() {
            match request {
                Request::ExecuteStreaming(_) => {
                    seen.push("execute_streaming");
                    if let Some(script) = script.take() {
                        script(&response_tx);
                    }
                }
                _ => seen.push("other"),
            }
        }
        seen
    });
    let session = Session {
        requests: request_tx,
        responses: response_rx,
        thread: None,
        poisoned: false,
        pending: None,
        head_reads: Vec::new(),
    };
    (session, stand_in)
}

#[test]
fn chunks_arrive_in_order_before_the_completed_command() {
    let (mut session, stand_in) = session_playing(|responses| {
        responses
            .send(Response::Output(OutputSource::Stdout, b"a".to_vec()))
            .expect("send");
        responses
            .send(Response::Output(OutputSource::Stderr, b"w".to_vec()))
            .expect("send");
        responses
            .send(Response::Output(OutputSource::Stdout, b"b".to_vec()))
            .expect("send");
        responses
            .send(Response::Executed(Box::new(completed())))
            .expect("send");
    });
    let mut seen = Vec::new();
    let result = session
        .execute_streaming(command(), Duration::from_secs(5), |source, bytes| {
            seen.push((source, bytes.to_vec()));
        })
        .expect("completes");
    assert_eq!(result.status, TerminalStatus::Exited(0));
    assert_eq!(
        seen,
        vec![
            (OutputSource::Stdout, b"a".to_vec()),
            (OutputSource::Stderr, b"w".to_vec()),
            (OutputSource::Stdout, b"b".to_vec()),
        ]
    );
    assert!(session.is_usable());
    drop(session);
    assert_eq!(
        stand_in.join().expect("stand-in"),
        vec!["execute_streaming"]
    );
}

#[test]
fn a_command_that_never_ends_is_bounded_by_the_whole_deadline_not_the_gap_between_chunks() {
    let (mut session, stand_in) = session_playing(|responses| {
        // A chunk every few milliseconds, forever, so no single gap ever exceeds the deadline.
        for _ in 0..200 {
            if responses
                .send(Response::Output(OutputSource::Stdout, b"x".to_vec()))
                .is_err()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let mut chunks = 0_u32;
    let result = session.execute_streaming(command(), Duration::from_millis(100), |_, _| {
        chunks += 1;
    });
    assert_eq!(result.err(), Some(SessionError::Gone));
    assert!(
        chunks > 0 && chunks < 100,
        "{chunks} chunks before the deadline"
    );
    assert!(!session.is_usable(), "an uncertain stream ends the session");
    drop(session);
    let _ = stand_in.join();
}

#[test]
fn an_unexpected_reply_poisons_the_session() {
    let (mut session, stand_in) = session_playing(|responses| {
        responses
            .send(Response::Ready {
                head_reads: Vec::new(),
            })
            .expect("send");
    });
    let result = session.execute_streaming(command(), Duration::from_secs(5), |_, _| {});
    assert_eq!(result.err(), Some(SessionError::Execute));
    assert!(!session.is_usable());
    drop(session);
    let _ = stand_in.join();
}

#[test]
fn a_failure_the_machine_reports_poisons_the_session_with_that_reason() {
    let (mut session, stand_in) = session_playing(|responses| {
        responses
            .send(Response::Failed(SessionError::Gone))
            .expect("send");
    });
    let result = session.execute_streaming(command(), Duration::from_secs(5), |_, _| {});
    assert_eq!(result.err(), Some(SessionError::Gone));
    assert!(!session.is_usable());
    drop(session);
    let _ = stand_in.join();
}

#[test]
fn a_poisoned_session_refuses_a_streaming_command_without_sending_it() {
    let (mut session, stand_in) = session_playing(|_| {});
    session.poisoned = true;
    let result = session.execute_streaming(command(), Duration::from_secs(5), |_, _| {});
    assert_eq!(result.err(), Some(SessionError::Poisoned));
    drop(session);
    assert!(stand_in.join().expect("stand-in").is_empty());
}
