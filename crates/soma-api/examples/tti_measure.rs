//! Measures the three create paths against machines this host really boots.
//!
//! Run it on a host with `/dev/kvm`, a prepared Generation for the reference it names, and a
//! scratch state root of its own:
//!
//! ```sh
//! SOMA_GENERATION_STORE=/srv/miosa/soma-qualification-fe3d01f/native-prepared \
//! SOMA_HEAD_DIR=/srv/miosa/soma-qualification-fe3d01f/native-heads \
//! SOMA_HEAD_SHARDS=16 SOMA_PREWARM_REFERENCE=node:22 SOMA_PREWARM_MEMORY_MIB=1024 \
//! SOMA_PREPARED_MACHINES=34 \
//! cargo run --release -p soma-api --example tti_measure -- /srv/miosa/tti-work/api/state
//! ```
//!
//! What it drives is [`Runner::handle`], the entry point the HTTP transport calls once it has
//! read a request; the wire above it is covered by the integration tests under `tests/runner`.
//! Every number it prints as server-side comes from the runner's own `Server-Timing` and body
//! fields, and each path's round trips are printed beside its totals so a reader can add the
//! network to one: on the wire a path costs one round trip per request it makes.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use sha2::{Digest, Sha256};
use soma_api::{
    ApiError, FacadePool, LocalFacade, SandboxFacade,
    runner::{
        Backend, FacadeOpener, Journal, KeyTable, Runner, RunnerConfig,
        feed::FeedEvent,
        service::{RunnerRequest, RunnerResponse},
    },
};
use soma_local::{BackendSelection, LocalRuntimeConfig};

/// The key every request carries, and the tenant it belongs to.
const TOKEN: &str = "msk_measure_tti";
const TENANT: &str = "0b0c3a52-6a7e-4d39-9f1e-3c4d5e6f7a8b";
/// The command every flow runs: the one a time-to-interactive benchmark runs first.
const COMMAND: &str = "node -v";
/// How many facades, and so how many concurrent creates, this measurement opens.
const WORKERS: usize = 48;
/// How many creates each measurement takes at one concurrency.
const ROUNDS_AT_ONE: usize = 30;
const ROUNDS_AT_CONCURRENCY: usize = 2;
const CONCURRENT: usize = 32;

/// One measured create, as the runner itself reported it.
struct Measured {
    /// How long the runner took to answer, from the caller handing it the request.
    total_us: u64,
    /// The `Server-Timing` segments of the create's own answer.
    create: Timing,
    /// The `Server-Timing` segments of the command's answer, for the flows that send one.
    command: Option<Timing>,
    /// The `x-miosa-soma-server-tti-us` the command's answer carried: how long the command took.
    command_us: Option<u64>,
    /// What the create body reported as the command's own duration, for a create-and-run.
    run_ms: Option<u64>,
}

/// The segments of one answer, in microseconds, and the body's own create time.
#[derive(Clone, Copy, Debug, Default)]
struct Timing {
    pool_us: u64,
    exec_us: u64,
    ready_us: u64,
    wait_us: Option<u64>,
    /// The `run` segment: the part of `exec` a create-and-run's command spent.
    run_us: Option<u64>,
    /// The body's `create_ms`, which is the `pool` and `exec` the same answer reports.
    create_ms: u64,
}

impl Timing {
    fn of(answer: &RunnerResponse) -> Self {
        let header = answer
            .headers
            .iter()
            .find(|(name, _)| *name == "server-timing")
            .map(|(_, value)| value.as_str())
            .expect("every runner answer carries a server-timing header");
        let named = |wanted: &str| -> Option<u64> {
            for entry in header.split(',') {
                let Some((name, value)) = entry.split_once(";dur=") else {
                    continue;
                };
                if name != wanted {
                    continue;
                }
                let (whole, fraction) = value.split_once('.')?;
                let whole: u64 = whole.parse().ok()?;
                let fraction: u64 = fraction.parse().ok()?;
                return Some(whole * 1_000 + fraction);
            }
            None
        };
        Self {
            pool_us: named("pool").unwrap_or(0),
            exec_us: named("exec").unwrap_or(0),
            ready_us: named("ready").unwrap_or(0),
            wait_us: named("wait"),
            run_us: named("run"),
            create_ms: answer_body(answer)["create_ms"].as_u64().unwrap_or(0),
        }
    }
}

/// One answer's body, decoded.
fn answer_body(answer: &RunnerResponse) -> serde_json::Value {
    serde_json::from_slice(&answer.body).unwrap_or(serde_json::Value::Null)
}

/// Milliseconds, with the microseconds the header carries.
fn millis(micros: u64) -> String {
    format!("{}.{:03}", micros / 1_000, micros % 1_000)
}

/// The median of one set of samples, in microseconds.
fn median(samples: &mut [u64]) -> u64 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// The 90th percentile of one set of samples, in microseconds.
fn ninetieth(samples: &mut [u64]) -> u64 {
    samples.sort_unstable();
    samples[(samples.len() * 9) / 10]
}

/// One line of the table: what one flow cost, as the caller saw it and as the runner reported it.
fn report(label: &str, totals: &mut [u64], detail: &str) {
    println!(
        "  {label:<22} n={:<4} total p50={:>10} ms  p90={:>10} ms  {detail}",
        totals.len(),
        millis(median(totals)),
        millis(ninetieth(totals)),
    );
}

/// The runner's configuration for a measurement: the reference this host has a Generation for,
/// and the shape that Generation was prepared at.
///
/// The TLS, control-plane and journal sections are parsed and never used: this harness drives
/// [`Runner::handle`] directly rather than the transport, so nothing here opens a connection.
fn config(journal: &Path, workers: usize) -> RunnerConfig {
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
            // The prepared Generation has no network, and this host has no broker to give it
            // one. Left at its default, `public_egress` would add network access to the shape
            // and the launch would be refused as a shape nothing was prepared for.
            "public_egress": false,
            // The shape the prepared Generation is built for, and so the one a launch can claim a
            // prepared machine with. Anything else takes the on-demand path and measures a cold
            // boot instead, which the `ready` column would show.
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
        // This measurement makes a burst of creates and commands from one key, which is exactly
        // what a per-key budget is there to bound rather than a property of the paths it compares.
        "rate_per_second": 100_000
    });
    RunnerConfig::parse(&serde_json::to_vec(&document).expect("encode")).expect("valid config")
}

/// Opens the local runtime the runner serves, with its machines hosted in a process of their own.
fn opener(state_root: &Path, workers: usize) -> FacadeOpener {
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
fn seed_keys(keys: &KeyTable) {
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
fn request(method: http::Method, path: &str, body: &str) -> RunnerRequest {
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

/// Creates one sandbox with `body`, runs the command the flow asks for, and destroys it.
async fn one_create(runner: &Runner, body: &str, second_call: bool) -> Measured {
    let started = Instant::now();
    let created = runner
        .handle(request(http::Method::POST, "/api/v1/sandboxes", body))
        .await;
    assert_eq!(created.status, 201, "create: {}", String::from_utf8_lossy(&created.body));
    let create_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let create = Timing::of(&created);
    let id = answer_body(&created)["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let mut command_us = None;
    let (command, total_us) = if second_call {
        let started = Instant::now();
        let executed = runner
            .handle(request(
                http::Method::POST,
                &format!("/api/v1/sandboxes/{id}/exec"),
                &serde_json::json!({"command": COMMAND}).to_string(),
            ))
            .await;
        assert_eq!(
            executed.status,
            200,
            "exec: {}",
            String::from_utf8_lossy(&executed.body)
        );
        assert!(
            !answer_body(&executed)["data"]["stdout"]
                .as_str()
                .unwrap_or("")
                .is_empty(),
            "the command produced output: {}",
            String::from_utf8_lossy(&executed.body)
        );
        command_us = executed
            .headers
            .iter()
            .find(|(name, _)| *name == "x-miosa-soma-server-tti-us")
            .and_then(|(_, value)| value.parse().ok());
        let elapsed = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        (Some(Timing::of(&executed)), create_us + elapsed)
    } else {
        let ran = answer_body(&created);
        assert_eq!(ran["run"]["exit_code"], 0, "{ran}");
        assert!(
            !ran["run"]["stdout"].as_str().unwrap_or("").is_empty(),
            "the command produced output: {ran}"
        );
        (None, create_us)
    };

    let destroyed = runner
        .handle(request(
            http::Method::DELETE,
            &format!("/api/v1/sandboxes/{id}"),
            "",
        ))
        .await;
    assert_eq!(
        destroyed.status,
        200,
        "destroy: {}",
        String::from_utf8_lossy(&destroyed.body)
    );

    Measured {
        total_us,
        command_us,
        create,
        command,
        run_ms: answer_body(&created)["run"]["duration_ms"].as_u64(),
    }
}

fn main() {
    // A hosted launch starts its machine host by re-entering this executable with
    // `machine-host`. Every binary that opens a hosted runtime has to answer here, before
    // anything else in it exists: without this the runtime would spawn a copy of this program
    // that measures instead of hosting, and every launch would fail.
    if std::env::args().nth(1).as_deref() == Some("machine-host") {
        std::process::exit(soma_local::host_machine(None));
    }
    measure();
}

#[tokio::main]
async fn measure() {
    let state_root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/soma-tti-measure".to_owned()),
    );
    std::fs::create_dir_all(&state_root).expect("the state root");
    if std::env::var("SOMA_PREPARED_MACHINES").is_err() {
        // Without this the host prepares one machine and every create but the first is a cold
        // boot, which would measure the wrong thing by an order of magnitude.
        println!("note: SOMA_PREPARED_MACHINES is unset, so most creates will be cold");
    }
    let journal = state_root.join("journal");
    std::fs::create_dir_all(&journal).expect("the journal directory");
    let config = Arc::new(config(&journal, WORKERS));
    let journal = Journal::open(&config.journal.directory, &config.runner).expect("journal");
    let runner = Arc::new(Runner::new(
        Arc::clone(&config),
        Backend::new(opener(&state_root, WORKERS)),
        journal,
    ));
    seed_keys(runner.keys());

    for (concurrency, rounds) in [(1, ROUNDS_AT_ONE), (CONCURRENT, ROUNDS_AT_CONCURRENCY)] {
        println!("\nconcurrency {concurrency}");
        let flows = [
            ("default + exec", "{}".to_owned(), true),
            (
                "early answer + exec",
                r#"{"wait_ready":false}"#.to_owned(),
                true,
            ),
            (
                "create and run",
                format!(r#"{{"run":{{"command":"{COMMAND}"}}}}"#),
                false,
            ),
        ];
        // Each flow's request count, which is what a network adds to its total: one round trip
        // per request, except that an early answer's command request is answered from inside the
        // wait its create's launch is already under, so its round trip is absorbed rather than
        // added.
        for (label, body, second_call) in flows {
            let mut totals = Vec::new();
            let mut creates = Vec::new();
            let mut launches = Vec::new();
            let mut waits = Vec::new();
            let mut commands = Vec::new();
            let mut runs = Vec::new();
            let mut bodies = Vec::new();
            for _round in 0..rounds {
                let mut tasks = Vec::new();
                for _ in 0..concurrency {
                    let body = body.clone();
                    let runner = Arc::clone(&runner);
                    tasks.push(tokio::spawn(async move {
                        one_create(&runner, &body, second_call).await
                    }));
                }
                for task in tasks {
                    let measured = task.await.expect("the caller ran");
                    totals.push(measured.total_us);
                    // What the create itself cost, and the launch's own ready phase.
                    creates.push(measured.create.pool_us + measured.create.exec_us);
                    launches.push(measured.create.ready_us);
                    if let Some(wait) = measured.command.as_ref().and_then(|timing| timing.wait_us) {
                        waits.push(wait);
                    }
                    if let Some(command) = measured.command_us {
                        commands.push(command);
                    }
                    if let Some(run) = measured.run_ms {
                        runs.push(run * 1_000);
                    }
                    // The `run` segment a create-and-run answers with: the part of `exec` its
                    // command spent, which the body's own figure also names.
                    if let Some(segment) = measured.create.run_us {
                        runs.push(segment);
                    }
                    bodies.push(measured.create.create_ms * 1_000);
                }
            }
            let optional = |samples: &mut Vec<u64>| -> String {
                if samples.is_empty() {
                    "none".to_owned()
                } else {
                    format!("{} ms", millis(median(samples)))
                }
            };
            let requests = if second_call { 2 } else { 1 };
            let detail = format!(
                "create p50={} us (body create_ms p50={} ms), ready p50={} us, waited p50={}, command p50={}, run p50={}, requests={requests}",
                median(&mut creates),
                median(&mut bodies),
                median(&mut launches),
                optional(&mut waits),
                optional(&mut commands),
                optional(&mut runs),
            );
            report(label, &mut totals, &detail);
        }
    }
}
