//! The exec body parameters the fast lane accepted, and the context a command runs with.

use super::params::ExecParams;

#[test]
fn exec_parameters_follow_the_fast_lane() {
    assert_eq!(
        ExecParams::parse(br#"{"command":"node -v"}"#).expect("accepted"),
        ExecParams {
            command: "node -v".to_owned(),
            timeout_ms: 30_000,
            context: soma::CommandContext::default()
        }
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":"ls","timeout":5,"cwd":null}"#)
            .expect("accepted")
            .timeout_ms,
        5_000
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":""}"#).map_err(|error| error.code),
        Err("MISSING_PARAM")
    );
    assert_eq!(
        ExecParams::parse(b"").map_err(|error| error.code),
        Err("MISSING_PARAM")
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":"ls","timeout":0}"#).map_err(|error| error.code),
        Err("INVALID_TIMEOUT")
    );
}

#[test]
fn exec_context_fields_are_parsed_and_bounded() {
    let parsed = ExecParams::parse(
        br#"{"command":"ls","cwd":"/work","user":"dev","env":{"B":"2","A":"1"}}"#,
    )
    .expect("accepted");
    assert_eq!(parsed.context.working_directory(), Some("/work"));
    assert_eq!(parsed.context.user(), Some("dev"));
    // The object decodes sorted, so one body is always one environment.
    assert_eq!(
        parsed.context.environment(),
        [
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "2".to_owned())
        ]
    );

    for body in [
        &br#"{"command":"ls","cwd":"relative"}"#[..],
        br#"{"command":"ls","cwd":7}"#,
        br#"{"command":"ls","user":""}"#,
        br#"{"command":"ls","env":["A=1"]}"#,
        br#"{"command":"ls","env":{"A":1}}"#,
        br#"{"command":"ls","env":{"A=B":"1"}}"#,
        br#"{"command":"ls","env":{"":"1"}}"#,
    ] {
        assert_eq!(
            ExecParams::parse(body).map_err(|error| error.code),
            Err("INVALID_PARAM"),
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    assert!(ExecParams::parse(br#"{"command":"ls","env":null,"cwd":null,"user":null}"#).is_ok());
}

#[test]
fn exec_standard_input_arrives_as_text_or_base64_and_is_bounded() {
    let text = ExecParams::parse(br#"{"command":"cat","stdin":"hello\n"}"#).expect("accepted");
    assert_eq!(text.context.stdin(), b"hello\n");
    let binary =
        ExecParams::parse(br#"{"command":"cat","stdin_base64":"AP8="}"#).expect("accepted");
    assert_eq!(binary.context.stdin(), [0_u8, 255]);

    let both = br#"{"command":"cat","stdin":"a","stdin_base64":"YQ=="}"#;
    let not_base64 = br#"{"command":"cat","stdin_base64":"!!!"}"#;
    let not_text = br#"{"command":"cat","stdin":7}"#;
    let over = format!(
        r#"{{"command":"cat","stdin":"{}"}}"#,
        "x".repeat(32 * 1024 + 1)
    );
    for body in [&both[..], not_base64, not_text, over.as_bytes()] {
        assert_eq!(
            ExecParams::parse(body).map_err(|error| error.code),
            Err("INVALID_PARAM")
        );
    }
    let at_bound = format!(r#"{{"command":"cat","stdin":"{}"}}"#, "x".repeat(32 * 1024));
    assert!(ExecParams::parse(at_bound.as_bytes()).is_ok());
}
