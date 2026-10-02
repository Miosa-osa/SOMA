//! Shared fixtures: tokens, certificates, configuration, and the runner launcher.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use sha2::{Digest, Sha256};
use soma::ExecutionReceipt;
use soma_api::runner::{self, FacadeOpener, RunnerConfig};

use crate::control_plane::ControlPlaneState;

pub(crate) const TOKEN: &str = "msk_us_runner_integration";
pub(crate) const OTHER_TOKEN: &str = "msk_us_never_published";
pub(crate) const TENANT: &str = "0b0c3a52-6a7e-4d39-9f1e-3c4d5e6f7a8b";
pub(crate) const RECEIPT: &str = include_str!("../fixtures/receipt.json");

pub(crate) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runner")
}

pub(crate) fn receipt() -> ExecutionReceipt {
    serde_json::from_str(RECEIPT).expect("the retained receipt is valid")
}

pub(crate) fn hash_of(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _written = write!(hex, "{byte:02x}");
            hex
        })
}

pub(crate) async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(crate) fn roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in CertificateDer::pem_file_iter(fixtures().join("ca.pem")).expect("CA") {
        roots.add(certificate.expect("CA cert")).expect("add CA");
    }
    roots
}

pub(crate) fn chain(name: &str) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_file_iter(fixtures().join(name))
        .expect("chain")
        .collect::<Result<_, _>>()
        .expect("certs")
}

pub(crate) fn key(name: &str) -> PrivateKeyDer<'static> {
    PrivateKeyDer::from_pem_file(fixtures().join(name)).expect("key")
}

pub(crate) fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

// ---------------------------------------------------------------- runner under test

pub(crate) fn config(
    control_plane: SocketAddr,
    journal: &Path,
    stale_seconds: u64,
) -> RunnerConfig {
    let fixtures = fixtures();
    let document = serde_json::json!({
        "runner": "miosa-host-03",
        "host_tag": "3",
        "listen": "127.0.0.1:0",
        "public_domain": "run-us.miosa.ai",
        "tls": {
            "certificate": fixtures.join("server.pem"),
            "private_key": fixtures.join("server-key.pem")
        },
        "control_plane": {
            "url": format!("https://127.0.0.1:{}", control_plane.port()),
            "server_name": "localhost",
            "ca": fixtures.join("ca.pem"),
            "certificate": fixtures.join("client.pem"),
            "private_key": fixtures.join("client-key.pem")
        },
        "journal": {"directory": journal, "batch_lines": 2},
        "launch": {
            "image": "docker.io/library/node:22",
            "shape": serde_json::to_value(soma::MachineShape::new(1, 512, 2_048).expect("shape")).expect("shape json"),
            "template_id": "miosa-sandbox-soma"
        },
        "admission": 8,
        "feed_stale_after_seconds": stale_seconds
    });
    RunnerConfig::parse(&serde_json::to_vec(&document).expect("encode")).expect("valid config")
}

/// Starts the runner the way the service does, from a plain thread: it builds its own runtime.
pub(crate) async fn start_runner(
    config: RunnerConfig,
    opener: FacadeOpener,
) -> runner::RunnerHandle {
    tokio::task::spawn_blocking(move || runner::spawn(config, opener))
        .await
        .expect("the start task")
        .expect("the runner starts")
}

pub(crate) fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "soma-runner-it-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&directory).expect("scratch");
    directory
}

pub(crate) fn snapshot(state: &ControlPlaneState, first_seq: u64) {
    state.send(&format!(r#"{{"seq":{first_seq},"kind":"snapshot_begin"}}"#));
    state.send(&format!(
        r#"{{"seq":{},"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":null}}"#,
        first_seq + 1,
        hash_of(TOKEN)
    ));
    state.send(&format!(
        r#"{{"seq":{},"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":null,"default_timeout_s":3600}}"#,
        first_seq + 2
    ));
    state.send(&format!(
        r#"{{"seq":{},"kind":"snapshot_end"}}"#,
        first_seq + 3
    ));
}
