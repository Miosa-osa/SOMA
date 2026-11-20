//! The platform's file routes through `Runner::handle`: routing, addressing, tenancy, and the
//! round trip a client makes, against the in-memory guest behind the flow double.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::{
    Runner,
    flow_tests::{Engine, body_of, call, created_id, runner},
};
use crate::runner::{files::MAX_JSON_WRITE_BODY_BYTES, ids::SandboxId, keys::tests::TOKEN};

async fn created(runner: &Runner) -> String {
    let created = call(runner, http::Method::POST, "/api/v1/sandboxes", "{}").await;
    assert_eq!(
        created.status,
        201,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    created_id(&created)
}

fn on(id: &str, rest: &str) -> String {
    format!("/api/v1/sandboxes/{id}/{rest}")
}

#[tokio::test]
async fn a_client_writes_lists_reads_renames_stats_and_deletes_a_file() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let contents: Vec<u8> = (0..=255).collect();
    let encoded = STANDARD.encode(&contents);

    let mkdir = call(
        &runner,
        http::Method::POST,
        &on(&id, "fs/mkdir"),
        r#"{"path":"/workspace/out","recursive":true}"#,
    )
    .await;
    assert_eq!(mkdir.status, 201);
    let write = call(
        &runner,
        http::Method::POST,
        &on(&id, "fs/write"),
        &format!(r#"{{"path":"/workspace/out/a.bin","content_base64":"{encoded}"}}"#),
    )
    .await;
    assert_eq!(
        write.status,
        201,
        "{}",
        String::from_utf8_lossy(&write.body)
    );

    let listed = call(
        &runner,
        http::Method::GET,
        &on(&id, "fs?path=%2Fworkspace%2Fout"),
        "",
    )
    .await;
    assert_eq!(listed.status, 200);
    assert_eq!(body_of(&listed)["files"][0]["name"], "a.bin");
    assert_eq!(body_of(&listed)["files"][0]["size_bytes"], 256);

    let read = call(
        &runner,
        http::Method::GET,
        &on(&id, "fs/read?path=/workspace/out/a.bin"),
        "",
    )
    .await;
    assert_eq!(read.status, 200);
    let read_back = STANDARD
        .decode(body_of(&read)["content_base64"].as_str().expect("base64"))
        .expect("decodes");
    assert_eq!(
        read_back, contents,
        "the bytes read back are the bytes written"
    );

    let renamed = call(
        &runner,
        http::Method::POST,
        &on(&id, "files/rename"),
        r#"{"from":"/workspace/out/a.bin","to":"/workspace/out/b.bin"}"#,
    )
    .await;
    assert_eq!(renamed.status, 200);

    let stat = call(
        &runner,
        http::Method::POST,
        &on(&id, "fs/stat"),
        r#"{"path":"/workspace/out/b.bin"}"#,
    )
    .await;
    assert_eq!(stat.status, 200);
    assert_eq!(body_of(&stat)["data"]["size"], 256);
    assert_eq!(body_of(&stat)["data"]["is_file"], true);

    let deleted = call(
        &runner,
        http::Method::DELETE,
        &on(&id, "fs?path=/workspace/out/b.bin"),
        "",
    )
    .await;
    assert_eq!(deleted.status, 200);
    let gone = call(
        &runner,
        http::Method::POST,
        &on(&id, "fs/stat"),
        r#"{"path":"/workspace/out/b.bin"}"#,
    )
    .await;
    assert_eq!(
        (gone.status, body_of(&gone)["code"].as_str()),
        (404, Some("NOT_FOUND"))
    );
    assert!(
        engine
            .guest
            .lock()
            .expect("guest")
            .nodes
            .keys()
            .all(|path| !path.contains(".soma-write-")),
        "no temporary sibling outlived its write"
    );
}

#[tokio::test]
async fn a_sandbox_the_caller_does_not_hold_is_not_found_and_another_runners_is_misdirected() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");

    let unknown = SandboxId::mint('3');
    let answer = call(
        &runner,
        http::Method::GET,
        &on(unknown.as_str(), "fs/read?path=/tmp/a"),
        "",
    )
    .await;
    assert_eq!(answer.status, 404);
    assert_eq!(body_of(&answer)["error"]["message"], "sandbox not found");

    let elsewhere = SandboxId::mint('9');
    let answer = call(
        &runner,
        http::Method::GET,
        &on(elsewhere.as_str(), "fs?path=/tmp"),
        "",
    )
    .await;
    assert_eq!(answer.status, 421);

    let answer = call(
        &runner,
        http::Method::GET,
        &on("not-a-uuid", "fs/read?path=/tmp/a"),
        "",
    )
    .await;
    assert_eq!(answer.status, 400);
    assert!(engine.guest.lock().expect("guest").log.is_empty());
}

#[tokio::test]
async fn an_unknown_file_action_is_no_route_and_an_unserved_one_says_so() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;

    let unknown = call(&runner, http::Method::POST, &on(&id, "fs/format"), "{}").await;
    assert_eq!(unknown.status, 404);
    let wrong_verb = call(&runner, http::Method::POST, &on(&id, "fs/read"), "{}").await;
    assert_eq!(wrong_verb.status, 404);
    let unserved = call(
        &runner,
        http::Method::POST,
        &on(&id, "fs/copy"),
        r#"{"from":"/tmp/a","to":"/tmp/b"}"#,
    )
    .await;
    assert_eq!(
        (unserved.status, body_of(&unserved)["code"].as_str()),
        (501, Some("FILE_ROUTE_UNSUPPORTED"))
    );
}

#[tokio::test]
async fn a_malformed_query_or_body_is_refused_before_the_guest_is_asked() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;

    let query = call(&runner, http::Method::GET, &on(&id, "fs/read?path=%zz"), "").await;
    assert_eq!(
        (query.status, body_of(&query)["code"].as_str()),
        (400, Some("INVALID_QUERY"))
    );
    let body = call(&runner, http::Method::POST, &on(&id, "fs/stat"), "[1]").await;
    assert_eq!(
        (body.status, body_of(&body)["code"].as_str()),
        (400, Some("INVALID_BODY"))
    );
    assert!(engine.guest.lock().expect("guest").log.is_empty());
}

#[tokio::test]
async fn a_trailing_separator_names_the_same_route_as_on_the_platform() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    engine.guest.lock().expect("guest").seed_directory("/tmp");
    let listed = call(&runner, http::Method::GET, &on(&id, "fs/?path=/tmp"), "").await;
    assert_eq!(listed.status, 200);
}

#[tokio::test]
async fn only_an_admitted_json_write_may_carry_a_large_body() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = SandboxId::mint('3');
    let write = on(id.as_str(), "fs/write");
    let bearer = format!("Bearer {TOKEN}");

    assert_eq!(
        runner.body_limit(&http::Method::POST, &write, Some(&bearer)),
        MAX_JSON_WRITE_BODY_BYTES
    );
    assert_eq!(
        runner.body_limit(&http::Method::POST, &write, Some("Bearer msk_unknown")),
        super::MAX_BODY_BYTES,
        "an unadmitted key never gets the larger allowance"
    );
    assert_eq!(
        runner.body_limit(&http::Method::POST, &on(id.as_str(), "exec"), Some(&bearer)),
        super::MAX_BODY_BYTES
    );
}
