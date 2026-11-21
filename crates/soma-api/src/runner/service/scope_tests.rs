//! Tenant scope on the runner: listings, the tenant default timeout, and suspension (C7).

use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

use soma::{BackendKind, MachineName, SandboxEntry, SandboxLiveness, SandboxPhase};

use super::{
    Runner,
    flow_tests::{Engine, call, runner},
};
use crate::runner::{
    ids::SandboxId,
    journal::tests::wait_for,
    keys::tests::{TENANT, event},
};

fn tenant_policy(runner: &Runner, seq: u64, tenant: &str, extra: &str) {
    runner
        .keys()
        .apply(
            &event(&format!(
                r#"{{"seq":{seq},"kind":"tenant_policy","tenant_id":"{tenant}","soma":true,{extra},"max_concurrent_share":null}}"#
            )),
            Instant::now(),
        )
        .expect("applies");
}

fn labelled(id: &SandboxId, tenant: &str) -> SandboxEntry {
    SandboxEntry::new(
        id.instance_id().expect("instance"),
        SandboxPhase::Active,
        BackendKind::LinuxKvm,
        Some(MachineName::parse(format!("t-{tenant}")).expect("label")),
        SandboxLiveness::Live,
    )
}

#[tokio::test]
async fn a_listing_holds_only_the_callers_sandboxes() {
    let engine = Arc::new(Engine::default());
    let mine = SandboxId::mint('3');
    let other_tenant = "5e6f7a8b-0000-4000-8000-000000000002";
    engine.listed.lock().expect("listed").extend([
        labelled(&mine, TENANT),
        labelled(&SandboxId::mint('3'), other_tenant),
    ]);
    let runner = runner(&engine, r#""*""#, "null");
    runner.recover_sandboxes().await;

    let listed = call(&runner, http::Method::GET, "/api/v1/sandboxes", "").await;
    assert_eq!(listed.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&listed.body).expect("JSON");
    assert_eq!(body["result"]["count"], 1);
    let instance = mine.instance_id().expect("instance");
    assert_eq!(
        body["result"]["sandboxes"][0]["instance_id"],
        instance.as_str()
    );
}

#[tokio::test]
async fn the_tenant_default_timeout_applies_and_suspension_reaps() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    tenant_policy(
        &runner,
        3,
        TENANT,
        r#""suspended":false,"default_timeout_s":120"#,
    );
    let created = call(&runner, http::Method::POST, "/api/v1/sandboxes", "").await;
    let body: serde_json::Value = serde_json::from_slice(&created.body).expect("JSON");
    assert_eq!(body["timeout_sec"], 120, "C7: the tenant default");

    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);
    tenant_policy(
        &runner,
        4,
        TENANT,
        r#""suspended":true,"default_timeout_s":120"#,
    );
    runner.reap().await;
    assert_eq!(
        engine.destroys.load(Ordering::SeqCst),
        1,
        "C7: a suspended tenant's sandboxes are destroyed by the next sweep"
    );
    wait_for(runner.journal(), 1);
    let journal = std::fs::read_to_string(runner.journal().path()).expect("journal");
    let line: serde_json::Value =
        serde_json::from_str(journal.lines().next().expect("a line")).expect("JSON");
    assert_eq!(line["kind"], "expire");
    assert_eq!(line["reason"], "suspended");
    assert_eq!(runner.sandboxes().live(), 0);
}

#[tokio::test]
async fn resident_lifecycle_only_reaches_the_facade_for_the_owning_tenant() {
    let engine = Arc::new(Engine::default());
    let mine = SandboxId::mint('3');
    let foreign = SandboxId::mint('3');
    engine.listed.lock().expect("listed").extend([
        labelled(&mine, TENANT),
        labelled(&foreign, "5e6f7a8b-0000-4000-8000-000000000002"),
    ]);
    let runner = runner(&engine, r#""*""#, "null");
    runner.recover_sandboxes().await;

    for operation in ["pause", "resume"] {
        let own = format!("/api/v1/sandboxes/{}/{operation}", mine.as_str());
        let other = format!("/api/v1/sandboxes/{}/{operation}", foreign.as_str());
        assert_eq!(
            call(&runner, http::Method::POST, &other, "").await.status,
            404
        );
        assert_eq!(
            call(&runner, http::Method::POST, &own, "").await.status,
            503
        );
        assert_eq!(
            call(&runner, http::Method::POST, &own, r#"{"unexpected":true}"#)
                .await
                .status,
            400
        );
    }
}

#[tokio::test]
async fn checkpoint_checks_tenant_ownership_before_forwarding() {
    let engine = Arc::new(Engine::default());
    let own = SandboxId::mint('3');
    let foreign = SandboxId::mint('3');
    engine.listed.lock().expect("listed").extend([
        labelled(&own, TENANT),
        labelled(&foreign, "5e6f7a8b-0000-4000-8000-000000000002"),
    ]);
    let runner = runner(&engine, r#""*""#, "null");
    runner.recover_sandboxes().await;
    let path = |id: &SandboxId| format!("/api/v1/sandboxes/{}/checkpoints", id.as_str());
    let body = r#"{"operation_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
    assert_eq!(
        call(&runner, http::Method::POST, &path(&foreign), body)
            .await
            .status,
        404
    );
    assert_eq!(
        call(&runner, http::Method::POST, &path(&own), body)
            .await
            .status,
        409
    );
    assert_eq!(
        call(&runner, http::Method::POST, &path(&own), "{}")
            .await
            .status,
        400
    );
}

#[tokio::test]
async fn restore_verifies_the_durable_tenant_after_losing_its_registry() {
    let engine = Arc::new(Engine::default());
    let own = SandboxId::mint('3');
    let foreign = SandboxId::mint('3');
    let foreign_tenant = "5e6f7a8b-0000-4000-8000-000000000002";
    engine
        .listed
        .lock()
        .expect("listed")
        .extend([labelled(&own, TENANT), labelled(&foreign, foreign_tenant)]);
    let runner = runner(&engine, r#""*""#, "1");
    assert_eq!(runner.sandboxes().live(), 0);
    let path = |id: &SandboxId| format!("/api/v1/sandboxes/{}/restore", id.as_str());
    let body = serde_json::json!({
        "operation_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "checkpoint_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "image": "node:22",
        "shape": soma::MachineShape::new(1, 1024, 256).unwrap(),
        "name": format!("t-{foreign_tenant}"),
    })
    .to_string();
    assert_eq!(
        call(&runner, http::Method::POST, &path(&foreign), &body)
            .await
            .status,
        404
    );
    assert!(
        engine
            .restore_names
            .lock()
            .expect("restore names")
            .is_empty()
    );
    assert_eq!(
        call(&runner, http::Method::POST, &path(&own), "{}")
            .await
            .status,
        400
    );
    assert_eq!(
        runner.sandboxes().live(),
        0,
        "failed attempt returns its reservation"
    );
    assert_eq!(
        call(&runner, http::Method::POST, &path(&own), &body)
            .await
            .status,
        200
    );
    assert_eq!(runner.sandboxes().live(), 1);
    let seen = engine.restore_names.lock().expect("restore names");
    assert_eq!(
        seen.as_slice(),
        &[Some(MachineName::parse(format!("t-{TENANT}")).unwrap())]
    );
}
