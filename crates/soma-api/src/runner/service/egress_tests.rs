//! `network.egress` on a create: the per-sandbox network, independent of the host default.
//!
//! The agent sandbox is created `denied` and must stay denied however the host is configured, so
//! the cases here are the ones that could widen it: a host default that connects every shape, a
//! misspelled key or value, and the broader unrestricted network.

use soma::{Capabilities, DnsPolicy, EgressPolicy};

use super::{
    params::{CreateParams, EgressRequest},
    tests::{launch, shape},
};
use crate::runner::config::LaunchConfig;

/// A runner whose default shape reaches the public internet, as `public_egress: true` makes it.
fn connected_host() -> LaunchConfig {
    let mut config = LaunchConfig {
        public_egress: true,
        shape: shape(),
        ..launch()
    };
    config.apply_egress_default();
    assert_eq!(
        config.shape.capabilities().network_policy().egress(),
        EgressPolicy::PublicInternet,
        "the fixture host connects its default shape"
    );
    config
}

fn parse(body: &str, config: &LaunchConfig) -> Result<CreateParams, (u16, &'static str)> {
    CreateParams::parse(body.as_bytes(), 3_600, config, true)
        .map_err(|error| (error.status, error.code))
}

#[test]
fn a_create_that_names_no_network_keeps_the_host_default() {
    let config = connected_host();
    for body in ["{}", r#"{"network":null}"#] {
        let parsed = parse(body, &config).expect("accepted");
        assert_eq!(parsed.shape, config.shape, "{body}");
        assert_eq!(parsed.egress, None, "{body}");
    }
}

#[test]
fn denied_is_denied_on_a_host_that_connects_by_default() {
    let parsed = parse(r#"{"network":{"egress":"denied"}}"#, &connected_host()).expect("accepted");

    let policy = parsed.shape.capabilities().network_policy();
    assert_eq!(policy.egress(), EgressPolicy::Denied);
    assert_eq!(
        policy.dns(),
        &DnsPolicy::Denied,
        "denied means no DNS either"
    );
    assert_eq!(parsed.egress, Some(EgressRequest::Denied));
    // Only the network changed: it is still the machine the host was sized for.
    assert_eq!(
        parsed.shape.vcpu_count(),
        connected_host().shape.vcpu_count()
    );
    assert_eq!(
        parsed.shape.memory_mib(),
        connected_host().shape.memory_mib()
    );
}

#[test]
fn internet_connects_a_host_whose_default_is_isolated() {
    let isolated_host = LaunchConfig {
        public_egress: false,
        ..launch()
    };
    let parsed = parse(r#"{"network":{"egress":"internet"}}"#, &isolated_host).expect("accepted");

    assert_eq!(
        parsed.shape.capabilities().network_policy().egress(),
        EgressPolicy::PublicInternet
    );
    assert_eq!(
        parsed.shape.capabilities().network_policy().dns(),
        &DnsPolicy::System
    );
    assert_eq!(parsed.egress, Some(EgressRequest::Internet));
}

#[test]
fn unrestricted_is_refused_unless_the_operator_allows_it() {
    let body = r#"{"network":{"egress":"unrestricted"}}"#;
    assert_eq!(
        parse(body, &launch()).map(|_| ()),
        Err((403, "egress_not_permitted"))
    );

    let allowed = LaunchConfig {
        allow_unrestricted_egress: true,
        ..launch()
    };
    let parsed = parse(body, &allowed).expect("an operator that allows it serves it");
    assert_eq!(
        parsed.shape.capabilities().network_policy(),
        Capabilities::isolated()
            .with_unrestricted_network()
            .network_policy()
    );
    assert_eq!(parsed.egress, Some(EgressRequest::Unrestricted));
}

#[test]
fn a_malformed_network_is_refused_never_served_as_the_default() {
    let config = connected_host();
    for body in [
        r#"{"network":"denied"}"#,
        r#"{"network":[]}"#,
        r#"{"network":{}}"#,
        r#"{"network":{"egress":null}}"#,
        r#"{"network":{"egress":"Denied"}}"#,
        r#"{"network":{"egress":"public_internet"}}"#,
        r#"{"network":{"egress":"none"}}"#,
        r#"{"network":{"egres":"denied"}}"#,
        r#"{"network":{"egress":"denied","dns":true}}"#,
    ] {
        assert_eq!(
            parse(body, &config).map(|_| ()),
            Err((400, "INVALID_PARAM")),
            "{body}"
        );
    }
}

#[test]
fn only_a_non_denied_egress_needs_the_broker() {
    assert!(!EgressRequest::Denied.needs_broker());
    assert!(EgressRequest::Internet.needs_broker());
    assert!(EgressRequest::Unrestricted.needs_broker());
}

#[test]
fn windowed_needs_a_denied_network_and_is_a_boolean() {
    let config = connected_host();
    let parsed = parse(r#"{"network":{"egress":"denied","windowed":true}}"#, &config)
        .expect("a windowed denied network is accepted");
    assert!(parsed.windowed);
    assert_eq!(parsed.egress, Some(EgressRequest::Denied));
    assert_eq!(
        parsed.shape.capabilities().network_policy().egress(),
        EgressPolicy::Denied,
        "windowed starts denied, however the host connects by default"
    );

    let plain = parse(r#"{"network":{"egress":"denied","windowed":false}}"#, &config)
        .expect("accepted");
    assert!(!plain.windowed);
    assert!(!parse(r#"{"network":{"egress":"denied"}}"#, &config).expect("ok").windowed);

    for body in [
        r#"{"network":{"egress":"internet","windowed":true}}"#,
        r#"{"network":{"egress":"unrestricted","windowed":true}}"#,
        r#"{"network":{"egress":"denied","windowed":"yes"}}"#,
        r#"{"network":{"egress":"denied","windowed":1}}"#,
    ] {
        assert_eq!(
            parse(body, &config).map(|_| ()),
            Err((400, "INVALID_PARAM")),
            "{body}"
        );
    }
}
