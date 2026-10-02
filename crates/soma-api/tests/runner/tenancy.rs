//! The tenant comes from the key record only; a client-supplied `x-soma-tenant` does nothing.

use std::sync::{Arc, atomic::Ordering};

use crate::{
    clients::{Answer, create_when_ready, with_headers},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{TENANT, TOKEN, config, eventually, hash_of, scratch, start_runner},
};

const TOKEN_B: &str = "msk_us_tenant_b_key";
const TENANT_B: &str = "7d1e2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b";

async fn as_key(
    address: std::net::SocketAddr,
    token: &str,
    method: &str,
    path: &str,
    spoofed_tenant: &str,
) -> Answer {
    with_headers(
        address,
        b"h2",
        (method, path),
        Some(token),
        &[
            ("x-soma-tenant", spoofed_tenant),
            ("x-forwarded-for", "10.0.0.1"),
        ],
        "",
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spoofed_tenant_header_acts_as_the_key_tenant() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch("tenancy");
    let handle = start_runner(config(control_plane, &journal, 900), opener(&engine)).await;
    let address = handle.address;
    eventually("the feed connection", || !state.connections().is_empty()).await;
    state.send(r#"{"seq":1,"kind":"snapshot_begin"}"#);
    for (seq, token, tenant, key) in [(2, TOKEN, TENANT, "k-a"), (4, TOKEN_B, TENANT_B, "k-b")] {
        state.send(&format!(
            r#"{{"seq":{seq},"kind":"key_upsert","key_hash":"{}","key_id":"{key}","tenant_id":"{tenant}","user_id":null,"projects":"*","rate_per_s":null}}"#,
            hash_of(token)
        ));
        state.send(&format!(
            r#"{{"seq":{},"kind":"tenant_policy","tenant_id":"{tenant}","soma":true,"suspended":false,"max_concurrent_share":null,"default_timeout_s":3600}}"#,
            seq + 1
        ));
    }
    state.send(r#"{"seq":6,"kind":"snapshot_end"}"#);
    let mine = create_when_ready(address, "").await.body["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let theirs = with_headers(
        address,
        b"h2",
        ("POST", "/api/v1/sandboxes"),
        Some(TOKEN_B),
        &[],
        "",
    )
    .await;
    assert_eq!(theirs.status, 201);
    let theirs = theirs.body["id"].as_str().expect("id").to_owned();

    // Key A claiming to be tenant B reaches nothing of B's.
    let inspect_theirs = as_key(
        address,
        TOKEN,
        "GET",
        &format!("/api/v1/sandboxes/{theirs}"),
        TENANT_B,
    )
    .await;
    assert_eq!(inspect_theirs.status, 404);
    assert_eq!(inspect_theirs.body["error"]["code"], "NOT_FOUND");
    let destroy_theirs = as_key(
        address,
        TOKEN,
        "DELETE",
        &format!("/api/v1/sandboxes/{theirs}"),
        TENANT_B,
    )
    .await;
    assert_eq!(destroy_theirs.status, 404);
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);
    assert_eq!(engine.inspects.load(Ordering::SeqCst), 0);

    // The same spoofed header on A's own sandbox still acts as A, through the forwarded
    // inspect route soma-api already serves on loopback.
    let inspect_mine = as_key(
        address,
        TOKEN,
        "GET",
        &format!("/api/v1/sandboxes/{mine}"),
        TENANT_B,
    )
    .await;
    assert_eq!(inspect_mine.status, 200, "{}", inspect_mine.body);
    assert_eq!(inspect_mine.body["operation"], "sandbox.get");
    assert_eq!(engine.inspects.load(Ordering::SeqCst), 1);
}
