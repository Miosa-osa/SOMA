//! The command-context half: the extra fields an exec carries, and the streaming requests.
//!
//! They live apart from the plain request tests so each file stays inside the source budget.

use super::*;

fn context() -> crate::ExecuteContext {
    crate::ExecuteContext {
        environment: vec![
            (b"A".to_vec(), b"1".to_vec()),
            (b"EMPTY".to_vec(), Vec::new()),
            (b"SPACED".to_vec(), b"two words".to_vec()),
        ],
        working_directory: Some(b"/work dir".to_vec()),
        user: None,
        stdin: b"SECRETSTDIN".to_vec(),
    }
}

#[test]
fn a_command_without_a_context_still_encodes_as_the_original_word() {
    let text = Request::Execute(execute()).encode();
    assert!(text.starts_with("execute "));
    assert!(!text.contains("execute_in"));
    let plain = Request::decode(&text).expect("decodes");
    assert_eq!(plain, Request::Execute(execute()));
}

#[test]
fn a_context_round_trips_including_empty_values_and_spaces() {
    let request = Request::Execute(execute().with_context(context()));
    let text = request.encode();
    assert!(text.starts_with("execute_in "));
    assert!(text.len() < MAX_REQUEST_BYTES);
    assert_eq!(Request::decode(&text).expect("decodes"), request);
}

#[test]
fn an_optional_user_and_directory_round_trip_when_present_and_absent() {
    for (directory, user) in [
        (None, None),
        (Some(b"/w".to_vec()), Some(b"dev".to_vec())),
        (None, Some(b"dev".to_vec())),
    ] {
        let mut context = context();
        context.working_directory = directory;
        context.user = user;
        let request = Request::Execute(execute().with_context(context));
        assert_eq!(
            Request::decode(&request.encode()).expect("decodes"),
            request
        );
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_context_the_guest_would_refuse_is_refused_at_decode() {
    for mutate in [
        (|c: &mut crate::ExecuteContext| c.working_directory = Some(b"relative".to_vec()))
            as fn(&mut crate::ExecuteContext),
        |c| c.environment.push((b"A=B".to_vec(), b"v".to_vec())),
        |c| c.environment.push((Vec::new(), b"v".to_vec())),
        |c| c.stdin = vec![0; 32 * 1024 + 1],
        |c| c.user = Some(Vec::new()),
    ] {
        let mut bad = context();
        mutate(&mut bad);
        let text = Request::Execute(execute().with_context(bad)).encode();
        if text.len() <= MAX_REQUEST_BYTES {
            assert_eq!(
                Request::decode(&text),
                Err(ControlError::InvalidValue("context")),
            );
        }
    }
}

#[test]
fn hostile_counts_are_refused_before_any_allocation_for_them() {
    let head = format!(
        "execute_in {} {} 5000 65536 x2f62696e2f74727565",
        hex(operation().as_bytes()),
        hex(instance().as_bytes())
    );
    assert_eq!(
        Request::decode(&format!("{head} 4294967295")),
        Err(ControlError::InvalidValue("argument count"))
    );
    assert_eq!(
        Request::decode(&format!("{head} 0 65")),
        Err(ControlError::InvalidValue("environment count"))
    );
    assert_eq!(
        Request::decode(&format!("{head} 0 1 x41")),
        Err(ControlError::MissingField("environment value"))
    );
    assert_eq!(
        Request::decode(&format!("{head} 0 0 - - x extra")),
        Err(ControlError::TrailingField)
    );
    // An untagged field is refused rather than read as hexadecimal.
    assert_eq!(
        Request::decode(&format!("{head} 0 0 - - 6162")),
        Err(ControlError::InvalidValue("stdin"))
    );
}

#[test]
fn debug_never_prints_a_value() {
    let shown = format!("{:?}", execute().with_context(context()));
    assert!(!shown.contains("two words") && !shown.contains("SECRETSTDIN"));
}
#[test]
fn a_streaming_request_is_a_word_of_its_own_and_round_trips() {
    let plain = Request::ExecuteStreaming(execute());
    let text = plain.encode();
    assert!(text.starts_with("execute_stream "));
    assert_eq!(Request::decode(&text).expect("decodes"), plain);

    let with_context = Request::ExecuteStreaming(execute().with_context(context()));
    assert_eq!(
        Request::decode(&with_context.encode()).expect("decodes"),
        with_context
    );
    // The same fields under the buffered word are a different request.
    let buffered = text.replacen("execute_stream", "execute_in", 1);
    assert_eq!(
        Request::decode(&buffered).expect("decodes"),
        Request::Execute(execute())
    );
}

#[test]
fn a_chunk_reply_round_trips_through_the_supervisors_decoder() {
    use crate::control::{Outcome, Reply};

    for (stream, bytes) in [
        (OutputStream::Stdout, b"hello \x00 \xff".to_vec()),
        (OutputStream::Stderr, vec![0_u8; 4096]),
    ] {
        let text = Reply::Chunk(stream, &bytes).encode();
        assert_eq!(Outcome::decode(&text), Ok(Outcome::Chunk(stream, bytes)));
    }
    for bad in [
        "chunk",
        "chunk stdout",
        "chunk sideways 00",
        "chunk stdout zz",
        "chunk stdout 00 00",
    ] {
        assert!(Outcome::decode(bad).is_err(), "{bad}");
    }
}
