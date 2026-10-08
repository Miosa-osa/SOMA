//! The create contract plan track T4 adds: an unknown field is refused by name, the 201 body
//! carries the server-side create time, and every field the platform defines still works.

use std::sync::{Arc, atomic::Ordering};

use crate::{
    clients::{create_when_ready, h2},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{TOKEN, config, eventually, scratch, snapshot, start_runner},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_create_names_an_unknown_field_instead_of_ignoring_it() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch("contract");
    let handle = start_runner(config(control_plane, &journal, 900), opener(&engine)).await;
    let address = handle.address;
    eventually("the feed connection", || !state.connections().is_empty()).await;
    snapshot(&state, 1);

    // An allowed create is unchanged, and its body now carries the server-side create time.
    let created = create_when_ready(address, "").await;
    assert_eq!(created.status, 201, "{}", created.text);
    assert!(created.body["create_ms"].is_u64(), "{}", created.text);
    assert_eq!(engine.launches.load(Ordering::SeqCst), 1);

    // A misspelled top-level field is a 400 that names it, and never reaches the facade.
    let typo = h2(
        address,
        "POST",
        "/api/v1/sandboxes",
        Some(TOKEN),
        r#"{"size":"xs","runtime_profil":"soma"}"#,
    )
    .await;
    assert_eq!(typo.status, 400, "{}", typo.text);
    assert_eq!(typo.body["error"]["code"], "INVALID_PARAM");
    assert_eq!(typo.body["error"]["details"]["field"], "runtime_profil");
    assert!(
        typo.body["error"]["message"]
            .as_str()
            .expect("message")
            .contains("runtime_profil"),
        "{}",
        typo.text
    );
    assert_eq!(
        engine.launches.load(Ordering::SeqCst),
        1,
        "a refused create never launches"
    );

    // Every field the platform defines is still accepted.
    let allowed = h2(
        address,
        "POST",
        "/api/v1/sandboxes",
        Some(TOKEN),
        r#"{"size":"xs","wait":true,"response_format":"compact","persistent":false,"runtime_profile":"soma","project_id":"p-1","region":"us","tags":{},"metadata":{"a":"b"},"disk_size_mb":4096,"idle_timeout_sec":0,"agent_runtime_profile_id":null,"revision":null}"#,
    )
    .await;
    assert_eq!(allowed.status, 201, "{}", allowed.text);
    assert_eq!(engine.launches.load(Ordering::SeqCst), 2);
}
