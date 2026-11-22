//! The `/v1` surface, generated from the router and the request bodies, held to its document.
//!
//! `docs/protocol/api-surface.md` holds the surface of the current API version and the digest of
//! every version's surface. A change to a route, a method, a request field, or the envelope
//! changes the generated text, and this test fails until the document records it under a new
//! version: a new minor for an addition, a new major for anything an older client cannot survive.

use std::fmt::Write as _;

use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};

use crate::{
    envelope::{ApiError, ENVELOPE_SCHEMA, failure_body},
    filesystem::FilesystemBody,
    handler::prewarm::PrewarmSetBody,
    handler::suspension::{ResumeBody, SuspendBody},
    http::request::{Method, Request},
    resources::ResourcesBody,
    route::{FilesystemOperation, Route, TerminalOperation, resolve},
    terminal::TerminalBody,
    version::API_VERSION,
    volumes::VolumeOperation,
    wire::{ControlBody, CreateSandboxBody, RunCommandBody},
};

const DOCUMENT: &str = include_str!("../../../../../docs/protocol/api-surface.md");
const INSTANCE: &str = "33333333333333333333333333333333";
const VOLUME: &str = "44444444444444444444444444444444";
const SUSPENSION: &str = "55555555555555555555555555555555";
const METHODS: [(Method, &str); 5] = [
    (Method::Get, "GET"),
    (Method::Post, "POST"),
    (Method::Put, "PUT"),
    (Method::Patch, "PATCH"),
    (Method::Delete, "DELETE"),
];

/// One value of every route, with every operation of the routes that carry one.
///
/// A new `Route` variant stops [`template`] compiling; add the variant here too.
fn every_route() -> Vec<Route> {
    let instance = || soma::InstanceId::new(INSTANCE).expect("a valid instance");
    let mut routes = vec![
        Route::Version,
        Route::Prewarm(false),
        Route::PrunePrepared,
        Route::Capacity,
        Route::CreateSandbox,
        Route::GetSandbox(instance()),
        Route::StopSandbox(instance()),
        Route::SuspendSandbox(instance()),
        Route::ResumeSuspension(SUSPENSION.to_owned()),
        Route::DestroySuspension(SUSPENSION.to_owned()),
        Route::RunCommand(instance()),
        Route::Resources(instance(), false),
        Route::Tunnel(instance(), 8080),
        Route::Volume(VolumeOperation::Read, Some(VOLUME.to_owned())),
    ];
    routes.extend(
        [
            FilesystemOperation::Read,
            FilesystemOperation::Write,
            FilesystemOperation::List,
            FilesystemOperation::Exists,
            FilesystemOperation::Remove,
            FilesystemOperation::MakeDirectory,
            FilesystemOperation::Stat,
            FilesystemOperation::Rename,
            FilesystemOperation::ReadRange,
            FilesystemOperation::WriteRange,
            FilesystemOperation::Chmod,
        ]
        .map(|operation| Route::Filesystem(instance(), operation)),
    );
    routes.extend(
        [
            TerminalOperation::Open,
            TerminalOperation::Write,
            TerminalOperation::Read,
            TerminalOperation::Resize,
            TerminalOperation::Close,
        ]
        .map(|operation| Route::Terminal(instance(), operation)),
    );
    routes.extend(
        [
            VolumeOperation::Attach,
            VolumeOperation::Detach,
            VolumeOperation::Destroy,
        ]
        .map(|operation| Route::Volume(operation, None)),
    );
    routes
}

/// The path template of a route, with `{instance}`, `{port}`, `{volume}`, and `{suspension}` for
/// its values.
fn template(route: &Route) -> String {
    match route {
        Route::Version => "/v1/version".to_owned(),
        Route::Prewarm(_) => "/v1/prewarm".to_owned(),
        Route::PrunePrepared => "/v1/prepared/prune".to_owned(),
        Route::Capacity => "/v1/capacity".to_owned(),
        Route::CreateSandbox | Route::ListSandboxes => "/v1/sandboxes".to_owned(),
        Route::GetSandbox(_) | Route::DestroySandbox(_) => "/v1/sandboxes/{instance}".to_owned(),
        Route::StopSandbox(_) => "/v1/sandboxes/{instance}/stop".to_owned(),
        Route::SuspendSandbox(_) => "/v1/sandboxes/{instance}/suspend".to_owned(),
        Route::ResumeSuspension(_) => "/v1/suspensions/{suspension}/resume".to_owned(),
        Route::DestroySuspension(_) => "/v1/suspensions/{suspension}".to_owned(),
        Route::RunCommand(_) => "/v1/sandboxes/{instance}/commands".to_owned(),
        Route::Filesystem(_, operation) => {
            format!("/v1/sandboxes/{{instance}}/filesystem/{}", operation.name())
        }
        Route::Terminal(_, operation) => {
            format!("/v1/sandboxes/{{instance}}/terminal/{}", operation.name())
        }
        Route::Tunnel(_, _) => "/v1/sandboxes/{instance}/tunnel/{port}".to_owned(),
        Route::Resources(_, _) => "/v1/sandboxes/{instance}/resources".to_owned(),
        Route::Volume(VolumeOperation::Read, _) => "/v1/volumes/{volume}".to_owned(),
        Route::Volume(operation, _) => format!(
            "/v1/volumes/{}",
            operation.name().trim_start_matches("volume.")
        ),
    }
}

fn concrete(template: &str) -> String {
    template
        .replace("{instance}", INSTANCE)
        .replace("{port}", "8080")
        .replace("{volume}", VOLUME)
        .replace("{suspension}", SUSPENSION)
}

/// The fields a request body accepts, read from the refusal of an unknown field.
fn fields<T: DeserializeOwned>() -> String {
    let error = serde_json::from_str::<T>(r#"{"surface-probe":0}"#)
        .err()
        .expect("every recorded body refuses unknown fields");
    let message = error.to_string();
    let listed = message
        .split_once("expected ")
        .map(|(_, rest)| rest.split(" at line").next().unwrap_or_default())
        .expect("serde names the fields it expected");
    listed
        .trim_start_matches("one of ")
        .split(", ")
        .map(|field| field.trim_matches('`'))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The surface of this build, one fact per line.
fn surface() -> String {
    let mut lines = vec![format!("api {API_VERSION}")];
    let envelope: serde_json::Value =
        serde_json::from_slice(&failure_body("request", ApiError::not_found()))
            .expect("an envelope");
    let keys: Vec<&str> = envelope
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    lines.push(format!("envelope {ENVELOPE_SCHEMA} {}", keys.join(" ")));
    let mut routes = Vec::new();
    for route in every_route() {
        let path = template(&route);
        for (method, name) in METHODS {
            let request = Request::from_parts(method, concrete(&path), Vec::new(), Vec::new());
            if let Ok(resolved) = resolve(&request) {
                assert_eq!(
                    template(&resolved),
                    path,
                    "{name} {path} resolves elsewhere"
                );
                routes.push(format!("route {name} {path} {}", resolved.operation()));
            }
        }
    }
    routes.sort();
    routes.dedup();
    lines.extend(routes);
    for (name, accepted) in [
        ("ControlBody", fields::<ControlBody>()),
        ("CreateSandboxBody", fields::<CreateSandboxBody>()),
        ("FilesystemBody", fields::<FilesystemBody>()),
        ("PrewarmSetBody", fields::<PrewarmSetBody>()),
        ("PrewarmTarget", fields::<soma_local::PrewarmTarget>()),
        ("PruneRequest", fields::<soma_local::PruneRequest>()),
        ("ResourcesBody", fields::<ResourcesBody>()),
        ("ResumeBody", fields::<ResumeBody>()),
        ("RunCommandBody", fields::<RunCommandBody>()),
        ("SuspendBody", fields::<SuspendBody>()),
        ("TerminalBody", fields::<TerminalBody>()),
    ] {
        lines.push(format!("body {name} {accepted}"));
    }
    lines.join("\n") + "\n"
}

fn digest(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    let hex = hash.iter().fold(String::new(), |mut hex, byte| {
        // Writing to a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    format!("sha256:{hex}")
}

/// The fenced block under `## Surface`.
fn documented_surface() -> String {
    let section = DOCUMENT
        .split_once("\n## Surface\n")
        .expect("the document has a Surface section")
        .1;
    let block = section
        .split_once("```text\n")
        .expect("the Surface section has a text block")
        .1;
    block
        .split_once("```")
        .expect("the text block ends")
        .0
        .to_owned()
}

/// The rows of the version table: `| 1.0 | sha256:... | ... |`.
fn documented_versions() -> Vec<(String, String)> {
    DOCUMENT
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            (cells.len() > 3 && cells[2].starts_with("sha256:"))
                .then(|| (cells[1].to_owned(), cells[2].to_owned()))
        })
        .collect()
}

#[test]
fn the_documented_surface_is_the_surface_this_build_serves() {
    let generated = surface();
    assert!(
        documented_surface() == generated,
        "the /v1 surface changed; record it in docs/protocol/api-surface.md under a new API \
         version (a minor for an addition, a major otherwise), then put this in its Surface \
         block:\n{generated}"
    );
}

#[test]
fn the_current_api_version_owns_the_current_surface_digest() {
    let versions = documented_versions();
    let (current, recorded) = versions.last().expect("the version table has a row");
    assert_eq!(
        *current,
        API_VERSION.to_string(),
        "the last row is API_VERSION"
    );
    assert_eq!(
        *recorded,
        digest(&surface()),
        "the surface changed without a new row in the version table"
    );
    let parsed: Vec<_> = versions
        .iter()
        .map(|(version, _)| crate::ApiVersion::parse(version).expect("a version"))
        .collect();
    assert!(
        parsed.windows(2).all(|pair| pair[0] < pair[1]),
        "versions only grow"
    );
    let mut digests: Vec<&String> = versions.iter().map(|(_, digest)| digest).collect();
    digests.sort();
    digests.dedup();
    assert_eq!(
        digests.len(),
        versions.len(),
        "every version has its own surface"
    );
}

#[test]
fn a_known_path_refuses_every_other_method_with_405() {
    // A path the router knows answers 405 to every method it does not serve, and 404 never.
    for route in every_route() {
        let path = concrete(&template(&route));
        for (method, name) in METHODS {
            let request = Request::from_parts(method, path.clone(), Vec::new(), Vec::new());
            if let Err(error) = resolve(&request) {
                assert_eq!(error.status(), 405, "{name} {path}");
            }
        }
    }
}
