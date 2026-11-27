//! The `/v1` surface, generated from the router and the request bodies, held to its document.
//!
//! `docs/product/reference/api-surface.md` holds the surface of the current API version and the digest of
//! every version's surface. A change to a route, a method, a request field, or the envelope
//! changes the generated text, and this test fails until the document records it under a new
//! version: a new minor for an addition, a new major for anything an older client cannot survive.

use serde::de::DeserializeOwned;

use crate::{
    envelope::{ApiError, ENVELOPE_SCHEMA, failure_body},
    filesystem::FilesystemBody,
    handler::prewarm::PrewarmSetBody,
    handler::suspension::{ResumeBody, SuspendBody},
    http::request::{Method, Request},
    resources::ResourcesBody,
    route::{
        FilesystemOperation, Route, TerminalOperation, TransferArtifact, TransferRoute, resolve,
    },
    terminal::TerminalBody,
    version::API_VERSION,
    volumes::VolumeOperation,
    wire::{ControlBody, CreateSandboxBody, RunCommandBody},
};
mod document;
mod paths;
use document::{digest, documented_surface, documented_versions};
use paths::{concrete, template};

const DOCUMENT: &str = include_str!("../../../../../docs/product/reference/api-surface.md");
const INSTANCE: &str = "33333333333333333333333333333333";
const VOLUME: &str = "44444444444444444444444444444444";
const SUSPENSION: &str = "55555555555555555555555555555555";
const IMPORT: &str = "66666666666666666666666666666666";
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
        Route::Autosize(instance(), false),
        Route::CreateSandbox,
        Route::GetSandbox(instance()),
        Route::StopSandbox(instance()),
        Route::SuspendSandbox(instance()),
        Route::ResumeSuspension(SUSPENSION.to_owned()),
        Route::DestroySuspension(SUSPENSION.to_owned()),
        Route::RunCommand(instance()),
        Route::PauseSandbox(instance()),
        Route::ResumeSandbox(instance()),
        Route::CheckpointSandbox(instance()),
        Route::DiscardCheckpoint(instance()),
        Route::RestoreSandbox(instance()),
        Route::ForkSandbox(instance()),
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
    let import = || soma::OperationId::new(IMPORT).expect("a valid import");
    routes.extend([
        Route::CheckpointTransfer(TransferRoute::Export),
        Route::CheckpointTransfer(TransferRoute::Commit(import())),
        Route::CheckpointTransfer(TransferRoute::Abort(import())),
    ]);
    routes.extend(
        [
            TransferArtifact::State,
            TransferArtifact::Memory,
            TransferArtifact::Overlay,
            TransferArtifact::Workspace,
        ]
        .map(|artifact| Route::CheckpointTransfer(TransferRoute::Receive(import(), artifact))),
    );
    routes
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
    let operation_field = format!(r#"{{"operation_id":"{INSTANCE}"}}"#);
    assert!(
        serde_json::from_str::<crate::handler::checkpoint::CheckpointBody>(&operation_field)
            .is_ok()
    );
    assert!(serde_json::from_str::<crate::handler::checkpoint::CheckpointBody>("{}").is_err());
    lines.push("body CheckpointBody operation_id(required)".to_owned());
    let digest_field = format!(r#"{{"state_digest":"sha256:{}"}}"#, "a".repeat(64));
    let discard_fields = format!(
        r#"{{"checkpoint_id":"{INSTANCE}","state_digest":"sha256:{}"}}"#,
        "a".repeat(64)
    );
    assert!(
        serde_json::from_str::<crate::handler::checkpoint::DiscardBody>(&discard_fields).is_ok()
    );
    let checkpoint_field = format!(r#"{{"checkpoint_id":"{INSTANCE}"}}"#);
    assert!(
        serde_json::from_str::<crate::handler::checkpoint::DiscardBody>(&checkpoint_field).is_err()
    );
    assert!(
        serde_json::from_str::<crate::handler::checkpoint::DiscardBody>(&digest_field).is_err()
    );
    for (name, accepted) in [
        ("AutosizeBody", fields::<crate::autosize::AutosizeBody>()),
        (
            "CheckpointCommitBody",
            fields::<crate::handler::transfer::CommitBody>(),
        ),
        (
            "CheckpointExportBody",
            fields::<crate::transfer::ExportBody>(),
        ),
        ("ControlBody", fields::<ControlBody>()),
        ("CreateSandboxBody", fields::<CreateSandboxBody>()),
        (
            "DiscardBody",
            "checkpoint_id(required) state_digest(required)".to_owned(),
        ),
        ("FilesystemBody", fields::<FilesystemBody>()),
        ("ForkBody", fields::<crate::handler::checkpoint::ForkBody>()),
        ("PrewarmSetBody", fields::<PrewarmSetBody>()),
        ("PrewarmTarget", fields::<soma_local::PrewarmTarget>()),
        ("PruneRequest", fields::<soma_local::PruneRequest>()),
        ("ResourcesBody", fields::<ResourcesBody>()),
        (
            "RestoreBody",
            fields::<crate::handler::checkpoint::RestoreBody>(),
        ),
        ("ResumeBody", fields::<ResumeBody>()),
        ("RunCommandBody", fields::<RunCommandBody>()),
        ("SuspendBody", fields::<SuspendBody>()),
        ("TerminalBody", fields::<TerminalBody>()),
    ] {
        lines.push(format!("body {name} {accepted}"));
    }
    lines.join("\n") + "\n"
}

#[test]
fn the_documented_surface_is_the_surface_this_build_serves() {
    let generated = surface();
    assert!(
        documented_surface() == generated,
        "the /v1 surface changed; record it in docs/product/reference/api-surface.md under a new API \
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
