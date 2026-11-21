use serde_json::Value;

use super::{API_VERSION, API_VERSION_HEADER, ApiVersion, handle_version, negotiate, protocols};
use crate::{
    envelope::ENVELOPE_SCHEMA,
    http::request::{Method, Request},
    tenant::TENANT_HEADER,
};

mod surface;

const COMMIT: &str = "2ec3789c4f1a8e9d0b1c2d3e4f5a6b7c8d9e0f1a";

fn request(path: &str, headers: &[(&str, &str)]) -> Request {
    let headers = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Request::from_parts(Method::Get, path.to_owned(), headers, Vec::new())
}

fn json(body: &[u8]) -> Value {
    serde_json::from_slice(body).expect("a JSON envelope")
}

#[test]
fn a_version_reads_as_major_or_major_and_minor() {
    assert_eq!(ApiVersion::parse("1"), Some(ApiVersion::new(1, 0)));
    assert_eq!(ApiVersion::parse("1.4"), Some(ApiVersion::new(1, 4)));
    assert_eq!(ApiVersion::parse("12.0"), Some(ApiVersion::new(12, 0)));
    for text in [
        "", "v1", "1.", ".1", "1.2.3", "+1", "1.-2", "one", "65536", " 1",
    ] {
        assert_eq!(ApiVersion::parse(text), None, "{text:?}");
    }
    assert_eq!(ApiVersion::new(1, 4).to_string(), "1.4");
}

#[test]
fn a_service_serves_its_own_major_up_to_its_own_minor() {
    let service = ApiVersion::new(1, 3);

    assert!(service.serves(ApiVersion::new(1, 0)));
    assert!(service.serves(ApiVersion::new(1, 3)));
    assert!(
        !service.serves(ApiVersion::new(1, 4)),
        "a newer minor adds what is missing"
    );
    assert!(
        !service.serves(ApiVersion::new(2, 0)),
        "a major is another surface"
    );
    assert!(!service.serves(ApiVersion::new(0, 9)));
}

#[test]
fn the_major_is_the_v1_of_the_envelope_and_of_every_path() {
    assert_eq!(
        ENVELOPE_SCHEMA,
        format!("soma.api.v{}", API_VERSION.major())
    );
    let route = crate::route::resolve(&request(
        &format!("/v{}/sandboxes", API_VERSION.major()),
        &[],
    ));
    assert!(route.is_ok(), "the path prefix is the major");
}

#[test]
fn a_request_without_the_header_is_served_as_before() {
    assert!(negotiate(&request("/v1/sandboxes", &[(TENANT_HEADER, "acme")])).is_none());
}

#[test]
fn a_request_needing_a_version_this_build_serves_is_admitted() {
    let current = API_VERSION.to_string();
    let major = API_VERSION.major().to_string();
    for stated in [current.as_str(), major.as_str(), " 1.0 "] {
        let stated_request = request(
            "/v1/sandboxes",
            &[(TENANT_HEADER, "acme"), (API_VERSION_HEADER, stated)],
        );
        assert!(negotiate(&stated_request).is_none(), "{stated:?}");
    }
}

#[test]
fn a_request_needing_another_version_is_refused_by_name() {
    let newer_minor = format!("{}.{}", API_VERSION.major(), API_VERSION.minor() + 1);
    let newer_major = (API_VERSION.major() + 1).to_string();
    for (stated, code) in [
        (newer_minor.as_str(), "api_version_unsupported"),
        (newer_major.as_str(), "api_version_unsupported"),
        ("v1", "invalid_api_version"),
    ] {
        let refused = negotiate(&request(
            "/v1/sandboxes",
            &[(TENANT_HEADER, "acme"), (API_VERSION_HEADER, stated)],
        ))
        .expect("refused");
        assert_eq!(refused.status, 400, "{stated}");
        let envelope = json(&refused.body);
        assert_eq!(envelope["error"]["code"], code, "{stated}");
        assert_eq!(envelope["error"]["retryable"], false);
    }
}

#[test]
fn an_unidentified_request_learns_nothing_about_versions() {
    let unidentified = request("/v1/sandboxes", &[(API_VERSION_HEADER, "9")]);
    assert!(
        negotiate(&unidentified).is_none(),
        "the handler refuses it as unidentified"
    );

    let response = handle_version(
        &request("/v1/version", &[]),
        &soma_version::Build::unknown(),
    );
    assert_eq!(response.status, 401);
    assert!(!String::from_utf8_lossy(&response.body).contains("release"));
}

#[test]
fn the_version_route_reports_the_api_the_build_and_every_protocol() {
    let build = soma_version::Build::from_environment(
        Some(COMMIT),
        Some("2026-11-20T18:04:11Z"),
        Some("clean"),
    );

    let response = handle_version(&request("/v1/version", &[(TENANT_HEADER, "ops")]), &build);

    assert_eq!(response.status, 200);
    let envelope = json(&response.body);
    assert_eq!(envelope["schema"], "soma.api.v1");
    assert_eq!(envelope["operation"], "service.version");
    let result = &envelope["result"];
    assert_eq!(result["api_version"], API_VERSION.to_string());
    assert_eq!(result["release"], soma_version::RELEASE);
    assert_eq!(result["commit"], COMMIT);
    assert_eq!(result["commit_short"], "2ec3789c4f1a");
    assert_eq!(result["commit_date"], "2026-11-20T18:04:11Z");
    assert_eq!(result["source"], "clean");
    assert_eq!(result["profile"], soma_version::PROFILE);
    assert_eq!(result["target"], soma_version::TARGET);
    let reported = result["protocols"].as_object().expect("an object");
    assert_eq!(reported.len(), protocols().len());
    for protocol in protocols() {
        assert_eq!(
            reported[protocol.name], protocol.version,
            "{}",
            protocol.name
        );
    }
    assert_eq!(reported["runner"], 1);
}
