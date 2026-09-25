use super::*;

fn plan(bytes: &[u8]) -> Result<CaptureWarmPlan, CaptureWarmError> {
    CaptureWarmPlan::decode(bytes)
}

#[test]
fn a_canonical_plan_round_trips_to_its_own_bytes() {
    let bytes = b"/usr/local/bin/node -v\n/usr/bin/python3 -c pass\n";
    let decoded = plan(bytes).expect("canonical plan");
    assert_eq!(decoded.encode(), bytes);
    assert_eq!(decoded.commands().len(), 2);
    assert_eq!(decoded.commands()[0].executable(), "/usr/local/bin/node");
    assert_eq!(decoded.commands()[0].arguments(), ["-v"]);
}

#[test]
fn an_empty_plan_is_not_a_plan() {
    assert_eq!(plan(b""), Err(CaptureWarmError::Empty));
    assert_eq!(
        CaptureWarmPlan::new(Vec::new()),
        Err(CaptureWarmError::Empty)
    );
}

#[test]
fn every_non_canonical_spelling_is_refused() {
    // Missing final newline, blank line, doubled space, trailing space, CRLF.
    assert_eq!(
        plan(b"/usr/local/bin/node -v"),
        Err(CaptureWarmError::NotCanonical)
    );
    assert!(plan(b"/usr/local/bin/node -v\n\n").is_err());
    assert!(plan(b"/usr/local/bin/node  -v\n").is_err());
    assert!(plan(b"/usr/local/bin/node -v \n").is_err());
    assert!(plan(b"/usr/local/bin/node -v\r\n").is_err());
}

#[test]
fn the_executable_must_be_absolute_without_parent_components() {
    assert_eq!(plan(b"node -v\n"), Err(CaptureWarmError::Executable));
    assert_eq!(plan(b"/usr/../bin/sh\n"), Err(CaptureWarmError::Executable));
}

#[test]
fn shell_syntax_quotes_and_control_bytes_are_refused() {
    for line in [
        "/bin/sh -c 'id'",
        "/bin/sh -c id;id",
        "/bin/echo $HOME",
        "/bin/echo a|b",
        "/bin/echo a>b",
        "/bin/echo \"a\"",
        "/bin/echo a\tb",
        "/bin/echo a\u{7}",
        "/bin/echo *",
    ] {
        let bytes = format!("{line}\n");
        assert_eq!(
            plan(bytes.as_bytes()),
            Err(CaptureWarmError::Argument),
            "{line}"
        );
    }
}

#[test]
fn bounds_are_enforced_on_commands_arguments_and_line_length() {
    let too_many = "/bin/true\n".repeat(MAX_WARM_COMMANDS + 1);
    assert_eq!(plan(too_many.as_bytes()), Err(CaptureWarmError::TooLarge));
    let many_arguments = format!("/bin/true{}\n", " a".repeat(MAX_WARM_ARGUMENTS));
    assert_eq!(
        plan(many_arguments.as_bytes()),
        Err(CaptureWarmError::TooLarge)
    );
    let long = format!("/{}\n", "a".repeat(MAX_WARM_LINE_BYTES));
    assert_eq!(plan(long.as_bytes()), Err(CaptureWarmError::TooLarge));
    let maximal = "/bin/true\n".repeat(MAX_WARM_COMMANDS);
    assert!(plan(maximal.as_bytes()).is_ok());
}
