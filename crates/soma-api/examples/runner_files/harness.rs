//! What the proof needs around the runner: the local runtime it serves, the key it accepts, and
//! the requests it is handed, written the way the service writes them.
//!
//! The TLS, control-plane and journal sections of the configuration are parsed and never used:
//! the proof drives `Runner::handle`, the entry point the transport calls once it has a request.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use sha2::{Digest, Sha256};
use soma_api::{
    ApiError, FacadePool, LocalFacade, SandboxFacade,
    runner::{
        FacadeOpener, KeyTable, RunnerConfig, body::StreamedBody, feed::FeedEvent,
        service::RunnerRequest,
    },
};
use soma_local::{BackendSelection, LocalRuntimeConfig};

/// Facades opened: one for the call in flight and one for the download running beside it.
pub(super) const WORKERS: usize = 2;

const TOKEN: &str = "msk_runner_files_proof";
const TENANT: &str = "0b0c3a52-6a7e-4d39-9f1e-3c4d5e6f7a8b";

/// The runner's configuration: `image` launched with no network, which a host without a network
/// broker can give it.
pub(super) fn config(journal: &Path, image: &str) -> RunnerConfig {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runner");
    let document = serde_json::json!({
        "runner": "runner-files-proof",
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
            "image": image,
            "template_id": "miosa-sandbox-soma",
            "public_egress": false,
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
        "admission": WORKERS
    });
    RunnerConfig::parse(&serde_json::to_vec(&document).expect("encode")).expect("valid config")
}

/// Opens the local KVM runtime the runner serves, with its machines hosted in processes of
/// their own, as the service opens it.
pub(super) fn opener(state_root: &Path) -> FacadeOpener {
    let local =
        LocalRuntimeConfig::discover(BackendSelection::Kvm, None, Some(state_root.to_path_buf()))
            .expect("the local sandbox runtime configuration")
            .with_hosted_machines(true);
    soma_local::start_prewarm(local.state_root(), WORKERS).expect("sterile machine hosts");
    let pool = FacadePool::open(WORKERS, || LocalFacade::open(local.clone()))
        .expect("the pool opens one runtime per worker");
    Arc::new(move || {
        pool.acquire_timeout(Duration::from_secs(30))
            .map(|lease| Box::new(lease) as Box<dyn SandboxFacade>)
            .ok_or_else(|| ApiError::new(503, "runtime_busy", "no facade within 30 s", true))
    })
}

/// Fills the key table with one key for one tenant, as a complete feed snapshot does.
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

/// One request, as the transport hands it to the runner, its path split from its query.
pub(super) fn request(method: http::Method, target: &str, body: &str) -> RunnerRequest {
    let (path, query) = target
        .split_once('?')
        .map_or((target, None), |(path, query)| {
            (path, Some(query.to_owned()))
        });
    RunnerRequest {
        method,
        path: path.to_owned(),
        query,
        authorization: Some(format!("Bearer {TOKEN}")),
        body: Bytes::from(body.to_owned()),
        received: Instant::now(),
        forwarded: false,
        upgrade: false,
        content_type: None,
        streamed: None,
    }
}

/// An upload request whose body arrives in `chunk`-byte pieces once the route takes it, as the
/// transport hands one on.
pub(super) fn upload(id: &str, form: Vec<u8>, boundary: &str, chunk: usize) -> RunnerRequest {
    let (feeder, streamed) = StreamedBody::channel();
    tokio::spawn(async move {
        let Some(sender) = feeder.wait().await else {
            return;
        };
        for piece in form.chunks(chunk) {
            if sender
                .send(Ok(Bytes::copy_from_slice(piece)))
                .await
                .is_err()
            {
                return;
            }
        }
    });
    let mut request = request(
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/fs/upload"),
        "",
    );
    request.content_type = Some(format!("multipart/form-data; boundary={boundary}"));
    request.streamed = Some(streamed);
    request
}
