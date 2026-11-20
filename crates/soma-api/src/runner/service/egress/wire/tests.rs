use super::*;

fn open(body: &str) -> Result<OpenRequest, PlatformError> {
    parse_open(body.as_bytes())
}

#[test]
fn a_window_request_names_its_id_and_a_bounded_ttl() {
    let parsed =
        open(r#"{"egress":"internet","ttl_ms":30000,"window_id":"call-1"}"#).expect("valid");
    assert_eq!(parsed.window_id, "call-1");
    assert_eq!(parsed.ttl, Duration::from_secs(30));
    assert!(parsed.dns, "dns defaults to on with the window");

    let without_dns =
        open(r#"{"egress":"internet","ttl_ms":1,"window_id":"c","dns":false}"#).expect("valid");
    assert!(!without_dns.dns);
}

#[test]
fn anything_else_is_refused_not_served_as_a_default() {
    for body in [
        "",
        "[]",
        r#"{"ttl_ms":30000,"window_id":"w"}"#,
        r#"{"egress":"denied","ttl_ms":30000,"window_id":"w"}"#,
        r#"{"egress":"unrestricted","ttl_ms":30000,"window_id":"w"}"#,
        r#"{"egress":"internet","window_id":"w"}"#,
        r#"{"egress":"internet","ttl_ms":0,"window_id":"w"}"#,
        r#"{"egress":"internet","ttl_ms":900001,"window_id":"w"}"#,
        r#"{"egress":"internet","ttl_ms":"30000","window_id":"w"}"#,
        r#"{"egress":"internet","ttl_ms":1000}"#,
        r#"{"egress":"internet","ttl_ms":1000,"window_id":"has space"}"#,
        r#"{"egress":"internet","ttl_ms":1000,"window_id":"w","dns":"yes"}"#,
        r#"{"egress":"internet","ttl_ms":1000,"window_id":"w","extra":1}"#,
    ] {
        let error = open(body)
            .err()
            .unwrap_or_else(|| panic!("{body:?} was accepted"));
        assert_eq!(error.status, 400, "{body:?}");
    }
}

#[test]
fn every_host_failure_has_its_own_stable_answer() {
    let answers: Vec<(u16, &str)> = [
        EgressSetFailure::NotFound,
        EgressSetFailure::NotWindowed,
        EgressSetFailure::NotActive,
        EgressSetFailure::FlipFailed,
        EgressSetFailure::Unavailable,
    ]
    .map(|failure| {
        let error = failure_error(failure);
        (error.status, error.code)
    })
    .to_vec();
    assert_eq!(
        answers,
        vec![
            (404, "NOT_FOUND"),
            (409, "egress_not_windowed"),
            (409, "egress_not_active"),
            (502, "egress_flip_failed"),
            (503, "egress_unavailable"),
        ]
    );
}
