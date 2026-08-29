use std::time::Duration;

use super::*;

fn prepared(limit: usize) -> PreparedCommand {
    PreparedCommand {
        request: Vec::new(),
        request_id: 7,
        challenge: [9; 32],
        timeout: Duration::from_secs(1),
        output_limit: limit,
    }
}

fn frame(kind: Kind, sequence: u32, payload: Vec<u8>) -> Frame {
    Frame {
        kind,
        request_id: 7,
        sequence,
        challenge: [9; 32],
        payload,
    }
}

fn terminal(kind: u8, value: i32, stdout: u32, stderr: u32) -> Vec<u8> {
    let mut payload = vec![0; 16];
    payload[0] = kind;
    payload[4..8].copy_from_slice(&value.to_be_bytes());
    payload[8..12].copy_from_slice(&stdout.to_be_bytes());
    payload[12..16].copy_from_slice(&stderr.to_be_bytes());
    payload
}

fn hello_frame() -> Frame {
    Frame {
        kind: Kind::Hello,
        request_id: 0,
        sequence: 0,
        challenge: [0; 32],
        payload: Vec::new(),
    }
}

#[test]
fn accepts_a_legal_fragmented_64_kib_combined_stream() {
    let mut collector = ResponseCollector::new(&prepared(64 * 1024));
    for sequence in 0..u32::try_from(64 * 1024).unwrap() {
        collector
            .accept(frame(Kind::Stdout, sequence, vec![b'x']))
            .unwrap();
    }
    let outcome = collector
        .accept(frame(
            Kind::Terminal,
            64 * 1024,
            terminal(0, 0, 64 * 1024, 0),
        ))
        .unwrap()
        .unwrap();
    assert_eq!(outcome.stdout.len(), 64 * 1024);
    assert_eq!(outcome.terminal, Arm64Terminal::Exited(0));
}

#[test]
fn rejects_wrong_identity_or_sequence_independently() {
    let mut wrong_request = frame(Kind::Stdout, 0, vec![1]);
    wrong_request.request_id += 1;
    let mut wrong_challenge = frame(Kind::Stdout, 0, vec![1]);
    wrong_challenge.challenge = [8; 32];
    let wrong_sequence = frame(Kind::Stdout, 1, vec![1]);
    for invalid in [wrong_request, wrong_challenge, wrong_sequence] {
        assert!(
            ResponseCollector::new(&prepared(20))
                .accept(invalid)
                .is_err()
        );
    }
}

#[test]
fn rejects_non_response_frames_after_the_handshake() {
    for kind in [Kind::Hello, Kind::Request] {
        assert!(
            ResponseCollector::new(&prepared(20))
                .accept(frame(kind, 0, Vec::new()))
                .is_err()
        );
    }
}

#[test]
fn rejects_empty_oversized_and_limit_exceeding_chunks() {
    for payload in [Vec::new(), vec![0; CHUNK_SIZE + 1]] {
        assert!(
            ResponseCollector::new(&prepared(CHUNK_SIZE + 1))
                .accept(frame(Kind::Stdout, 0, payload))
                .is_err()
        );
    }
    let mut collector = ResponseCollector::new(&prepared(4));
    collector
        .accept(frame(Kind::Stdout, 0, vec![1, 2]))
        .unwrap();
    assert!(
        collector
            .accept(frame(Kind::Stderr, 1, vec![3, 4, 5]))
            .is_err()
    );
}

#[test]
fn terminal_requires_exact_shape_counts_and_reserved_bytes() {
    for payload in [vec![0; 15], vec![0; 17]] {
        assert!(
            ResponseCollector::new(&prepared(20))
                .accept(frame(Kind::Terminal, 0, payload))
                .is_err()
        );
    }
    let mut reserved = terminal(0, 0, 0, 0);
    reserved[2] = 1;
    assert!(
        ResponseCollector::new(&prepared(20))
            .accept(frame(Kind::Terminal, 0, reserved))
            .is_err()
    );
    let mut collector = ResponseCollector::new(&prepared(20));
    collector
        .accept(frame(Kind::Stderr, 0, b"short".to_vec()))
        .unwrap();
    assert!(
        collector
            .accept(frame(Kind::Terminal, 1, terminal(0, 0, 0, 0)))
            .is_err()
    );
}

#[test]
fn terminal_rejects_unknown_or_noncanonical_outcomes() {
    for (kind, value) in [
        (6, 0),
        (0, 256),
        (1, 0),
        (1, 65),
        (2, 1),
        (3, 1),
        (4, 0),
        (4, 4096),
        (5, 0),
        (5, 4096),
    ] {
        assert!(
            ResponseCollector::new(&prepared(20))
                .accept(frame(Kind::Terminal, 0, terminal(kind, value, 0, 0)))
                .is_err()
        );
    }
}

#[test]
fn output_limit_requires_the_exact_retained_allowance() {
    let mut collector = ResponseCollector::new(&prepared(4));
    collector
        .accept(frame(Kind::Stdout, 0, vec![1, 2, 3]))
        .unwrap();
    assert!(
        collector
            .accept(frame(Kind::Terminal, 1, terminal(3, 0, 3, 0)))
            .is_err()
    );
}

#[test]
fn accepts_legal_terminal_boundaries_and_stream_attribution() {
    for (kind, value, expected) in [
        (0, 255, Arm64Terminal::Exited(255)),
        (1, 64, Arm64Terminal::Signaled(64)),
        (4, 4095, Arm64Terminal::ExecFailed(4095)),
        (5, 4095, Arm64Terminal::AgentFailed(4095)),
    ] {
        let outcome = ResponseCollector::new(&prepared(20))
            .accept(frame(Kind::Terminal, 0, terminal(kind, value, 0, 0)))
            .unwrap()
            .unwrap();
        assert_eq!(outcome.terminal, expected);
    }

    let mut collector = ResponseCollector::new(&prepared(6));
    collector
        .accept(frame(Kind::Stdout, 0, b"ab".to_vec()))
        .unwrap();
    collector
        .accept(frame(Kind::Stderr, 1, b"cd".to_vec()))
        .unwrap();
    collector
        .accept(frame(Kind::Stdout, 2, b"ef".to_vec()))
        .unwrap();
    let outcome = collector
        .accept(frame(Kind::Terminal, 3, terminal(0, 0, 4, 2)))
        .unwrap()
        .unwrap();
    assert_eq!(outcome.stdout, b"abef");
    assert_eq!(outcome.stderr, b"cd");
}

#[test]
fn hello_is_exact_and_carries_no_launch_secret() {
    assert!(validate_hello(&hello_frame()).is_ok());
    for invalid in [
        Frame {
            request_id: 1,
            ..hello_frame()
        },
        Frame {
            sequence: 1,
            ..hello_frame()
        },
        Frame {
            challenge: [1; 32],
            ..hello_frame()
        },
        Frame {
            payload: vec![0],
            ..hello_frame()
        },
        Frame {
            kind: Kind::Terminal,
            ..hello_frame()
        },
    ] {
        assert!(validate_hello(&invalid).is_err());
    }
}

#[test]
fn exactly_one_terminal_frame_is_accepted() {
    let mut collector = ResponseCollector::new(&prepared(20));
    assert!(
        collector
            .accept(frame(Kind::Terminal, 0, terminal(0, 0, 0, 0)))
            .unwrap()
            .is_some()
    );
    assert!(
        collector
            .accept(frame(Kind::Terminal, 0, terminal(0, 0, 0, 0)))
            .is_err()
    );
}
