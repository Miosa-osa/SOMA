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
    assert_eq!(runner.sandboxes().live(), 0);
}
