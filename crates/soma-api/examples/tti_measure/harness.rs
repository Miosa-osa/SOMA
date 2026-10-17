//! What the measurement needs around the runner: the local runtime it serves, the key it
//! accepts, and the requests it is handed.
//!
//! None of this is the subject of the measurement, and it is written the way the service writes
//! it so that the two create paths are measured on the runner the service runs rather than on one
//! made for the occasion. The TLS, control-plane and journal sections of the configuration are
//! parsed and never used: this harness drives `Runner::handle` directly rather than the transport.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use sha2::{Digest, Sha256};
use soma_api::{
    ApiError, FacadePool, LocalFacade, SandboxFacade,
    runner::{FacadeOpener, KeyTable, RunnerConfig, feed::FeedEvent, service::RunnerRequest},
};
use soma_local::{BackendSelection, LocalRuntimeConfig};

/// How many facades, and so how many concurrent creates, this measurement opens.
pub(super) const WORKERS: usize = 48;

/// The key every request carries, and the tenant it belongs to.
const TOKEN: &str = "msk_measure_tti";
const TENANT: &str = "0b0c3a52-6a7e-4d39-9f1e-3c4d5e6f7a8b";

/// The runner's configuration for a measurement: the reference this host has a Generation for,
/// and the shape that Generation was prepared at.
pub(super) fn config(journal: &Path, workers: usize) -> RunnerConfig {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runner");
    let document = serde_json::json!({
        "runner": "measure-tti",
        "host_tag": "1",
        "listen": "127.0.0.1:0",
        "public_domain": "run-us.miosa.ai",
        "tls": {
            "certificate": fixtures.join("server.pem"),
            "private_key": fixtures.join("server-key.pem")
        },
        "control_plane": {
            "url": "https://127.0.0.1:1",
            "server_name": "localhost",
            "ca": fixtures.join("ca.pem"),
            "certificate": fixtures.join("client.pem"),
            "private_key": fixtures.join("client-key.pem")
        },
        "journal": {"directory": journal, "batch_lines": 2},
        "launch": {
            "image": "node:22",
            "template_id": "miosa-sandbox-soma",
            // The prepared Generation has no network, and this host has no broker to give it one.
            // Left at its default, `public_egress` would add network access to the shape and the
            // launch would be refused as a shape nothing was prepared for.
            "public_egress": false,
            // The shape the prepared Generation is built for, and so the one a launch can claim a
            // prepared machine with. Anything else takes the on-demand path and measures a cold
            // boot instead, which the `ready` column of the table would show.
            "shape": {
                "capabilities": {
                    "network": {
                        "profile": {"mode": "disabled"},
                        "proxy": {"mode": "disabled"},
                        "dns": {"mode": "denied"},
                        "egress": "denied",
                        "guest_addresses": {
                            "ipv4": {"mode": "disabled"},
                            "ipv6": {"mode": "disabled"}
                        },
                        "published_ports": []
                    }
                },
                "memory_mib": 1024,
                "vcpu_count": 1,
                "storage_mib": 4096
            }
        },
        "admission": workers,
        // A measurement makes a burst of creates and commands from one key, which is what a
        // per-key budget exists to bound rather than a property of the paths it compares.
        "rate_per_second": 100_000
    });
    RunnerConfig::parse(&serde_json::to_vec(&document).expect("encode")).expect("valid config")
}

/// Opens the local runtime the runner serves, with its machines hosted in a process of their own.
pub(super) fn opener(state_root: &Path, workers: usize) -> FacadeOpener {
    let local =
        LocalRuntimeConfig::discover(BackendSelection::Kvm, None, Some(state_root.to_path_buf()))
            .expect("the local sandbox runtime configuration")
            .with_hosted_machines(true);
    soma_local::start_prewarm(local.state_root(), workers).expect("sterile machine hosts");
    let pool = FacadePool::open(workers, || LocalFacade::open(local.clone()))
        .expect("the pool opens one runtime per worker");
    let acquired = pool.clone();
    Arc::new(move || {
        acquired
            .acquire_timeout(Duration::from_secs(5))
            .map(|lease| Box::new(lease) as Box<dyn SandboxFacade>)
            .ok_or_else(runtime_busy)
    })
}

fn runtime_busy() -> ApiError {
    ApiError::new(
        503,
        "runtime_busy",
        "the local sandbox runtime is at its bounded request capacity",
        true,
    )
}

/// Fills a runner's key table with one key for one tenant, as a complete feed snapshot does.
pub(super) fn seed_keys(keys: &KeyTable) {
    let hash = Sha256::digest(TOKEN.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _written = write!(hex, "{byte:02x}");
            hex
        });
    let now = Instant::now();
    for line in [
        r#"{"seq":1,"kind":"snapshot_begin"}"#.to_owned(),
        format!(
            r#"{{"seq":2,"kind":"key_upsert","key_hash":"{hash}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":null}}"#
        ),
        format!(
            r#"{{"seq":3,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":null,"default_timeout_s":3600}}"#
        ),
        r#"{"seq":4,"kind":"snapshot_end"}"#.to_owned(),
    ] {
        keys.apply(&FeedEvent::parse(&line).expect("a feed line"), now)
            .expect("the snapshot applies");
    }
}

/// One request, as the transport would have handed it to the runner.
pub(super) fn request(method: http::Method, path: &str, body: &str) -> RunnerRequest {
    RunnerRequest {
        method,
        path: path.to_owned(),
        authorization: Some(format!("Bearer {TOKEN}")),
        body: Bytes::from(body.to_owned()),
        received: Instant::now(),
        forwarded: false,
        upgrade: false,
    }
}
