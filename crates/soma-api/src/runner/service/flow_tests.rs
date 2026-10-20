//! Request flows through the runner with a fake facade: recovery, expiry, and tenant scope.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use bytes::Bytes;
use soma::{
    BackendFailureKind, BackendKind, DestroyMachineRequest, ExecuteMachineRequest,
    ExecutionReceipt, FileMachineRequest, InspectMachineRequest, LaunchMachineRequest, MachineName,
    ManagedFailure, PtyMachineRequest, SandboxEntry, SandboxLiveness, SandboxPhase,
    StopMachineRequest, TerminalStatus,
};

use super::{Runner, RunnerRequest, RunnerResponse};
use crate::{
    facade::{
        CommandOutcome, FileOutcome, LifecycleOutcome, SandboxFacade, SandboxSnapshot,
        TerminalOutcome,
    },
    runner::{
        backend::Backend,
        config::{RunnerConfig, tests::document},
        ids::SandboxId,
        journal::{
            Journal,
            tests::{scratch, wait_for},
        },
        keys::tests::{TENANT, TOKEN, event, hash_of},
    },
};

mod fake;

const RECEIPT: &str = include_str!("../../../tests/fixtures/receipt.json");

#[derive(Default)]
pub(super) struct Engine {
    /// How many launches reached the facade.
    pub(super) launches: AtomicUsize,
    pub(super) destroys: AtomicUsize,
    /// How many commands reached the facade. A refused command must leave this where it was.
    pub(super) execs: AtomicUsize,
    /// A refusal the next command gets instead of running, so a test can drive the failure paths
    /// the engine reports after it has taken an operation.
    pub(super) exec_refusal: Mutex<Option<BackendFailureKind>>,
    pub(super) listed: Mutex<Vec<SandboxEntry>>,
    /// How long each command takes, for tests of commands that outlast the idle timeout.
    pub(super) exec_delay: Mutex<Duration>,
    /// How long each terminal call takes, as a long `read` with `wait_ms` would.
    pub(super) terminal_delay: Mutex<Duration>,
    /// Output a streaming command delivers one chunk at a time, with the command held open
    /// after the first chunk until the gate is released. Taken by the next streaming command.
    pub(super) stream_script: Mutex<Option<StreamScript>>,
    /// A launch holds here until the test releases it, so a create's answer can be observed while
    /// the launch it started is still in flight.
    pub(super) launch_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// The refusal the next launch gets instead of a machine, taken one at a time.
    pub(super) launch_refusal: Mutex<Option<BackendFailureKind>>,
    /// What the next command reports as its terminal status, taken one at a time.
    pub(super) exec_status: Mutex<Option<TerminalStatus>>,
    /// Every egress change that reached the facade, in order.
    pub(super) egress_calls: Mutex<Vec<soma_local::EgressSetRequest>>,
    /// What the next egress changes answer instead of succeeding, taken one at a time.
    pub(super) egress_script: Mutex<
        std::collections::VecDeque<
            Result<soma_local::EgressSetOutcome, soma_local::EgressSetFailure>,
        >,
    >,
}

/// Chunks for one streaming command and the gate between its first chunk and the rest.
pub(super) struct StreamScript {
    pub(super) chunks: Vec<(soma::OutputChannel, Vec<u8>)>,
    pub(super) gate: std::sync::mpsc::Receiver<()>,
}

struct Fake(Arc<Engine>);

/// A runner whose table holds `TOKEN` for `TENANT` with the given scope and cap.
pub(super) fn runner(engine: &Arc<Engine>, projects: &str, max_concurrent: &str) -> Runner {
    let config =
        RunnerConfig::parse(&serde_json::to_vec(&document()).expect("encode")).expect("config");
    let opener_engine = Arc::clone(engine);
    let backend = Backend::new(Arc::new(move || {
        Ok(Box::new(Fake(Arc::clone(&opener_engine))) as Box<dyn SandboxFacade>)
    }));
    let journal = Journal::open(&scratch("flow"), "miosa-host-03").expect("journal");
    let runner = Runner::new(Arc::new(config), backend, journal);
    let now = Instant::now();
    for line in [
        format!(
            r#"{{"seq":1,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":{projects},"rate_per_s":null}}"#,
            hash_of(TOKEN)
        ),
        format!(
            r#"{{"seq":2,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":{max_concurrent}}}"#
        ),
    ] {
        runner.keys().apply(&event(&line), now).expect("applies");
    }
    runner
}

pub(super) async fn call(
    runner: &Runner,
    method: http::Method,
    path: &str,
    body: &str,
) -> RunnerResponse {
    runner
        .handle(RunnerRequest {
            method,
            path: path.to_owned(),
            authorization: Some(format!("Bearer {TOKEN}")),
            body: Bytes::from(body.to_owned()),
            received: Instant::now(),
            forwarded: false,
            upgrade: false,
        })
        .await
}

pub(super) fn created_id(response: &RunnerResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON");
    body["id"].as_str().expect("id").to_owned()
}

/// The `Server-Timing` segments one answer reports, in microseconds.
///
/// The header carries milliseconds with three decimals, so every value here is exact rather than
/// rounded: a test can compare a segment with the body's own `create_ms` instead of with a
/// tolerance.
pub(super) fn timing_of(response: &RunnerResponse) -> std::collections::BTreeMap<String, u64> {
    let header = response
        .headers
        .iter()
        .find(|(name, _)| *name == "server-timing")
        .map(|(_, value)| value.as_str())
        .expect("every answer carries a server-timing header");
    // A metric with a description and no duration, such as `launch;desc="prepared"`, is a fact
    // about the request rather than a span of it, so it has no place in a map of durations.
    header
        .split(',')
        .filter_map(|segment| {
            let (name, value) = segment.split_once(";dur=")?;
            let (millis, micros) = value.split_once('.').expect("three decimals");
            Some((
                name.to_owned(),
                millis.parse::<u64>().expect("millis") * 1_000
                    + micros.parse::<u64>().expect("micros"),
            ))
        })
        .collect()
}

/// The decoded body of one answer.
pub(super) fn body_of(response: &RunnerResponse) -> serde_json::Value {
    serde_json::from_slice(&response.body).expect("the answer is JSON")
}

/// Sweeps until the expired sandbox is gone, or fails once the deadline passes.
///
/// A sweep compares wall-clock instants, so waiting on the condition rather than on a fixed margin
/// is what keeps these tests about expiry and not about how busy the machine running them is.
pub(super) async fn reap_until_destroyed(runner: &Runner, engine: &Engine) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.destroys.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the reaper never destroyed it");
        tokio::time::sleep(Duration::from_millis(10)).await;
        runner.reap().await;
    }
}

#[tokio::test]
async fn a_restarted_runner_readopts_the_sandboxes_its_tenants_created() {
    let engine = Arc::new(Engine::default());
    let id = SandboxId::mint('3');
    engine
        .listed
        .lock()
        .expect("listed")
        .push(SandboxEntry::new(
            id.instance_id().expect("instance"),
            SandboxPhase::Active,
            BackendKind::LinuxKvm,
            Some(MachineName::parse(format!("t-{TENANT}")).expect("label")),
            SandboxLiveness::Live,
        ));
    let runner = runner(&engine, r#""*""#, "null");
    runner.recover_sandboxes().await;

    let exec = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"false"}"#,
    )
    .await;
    assert_eq!(exec.status, 200);
    assert_eq!(
        exec.body,
        br#"{"data":{"exit_code":3,"stderr":"no","stdout":""}}"#.to_vec()
    );
    let journal = exec.journal.expect("an exec is paperwork");
    assert_eq!(journal.exit_code, Some(3));
}

#[tokio::test]
async fn an_expired_sandbox_is_destroyed_by_the_reaper() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    // A sandbox with time left is not swept. Its timeout is an hour rather than a second because
    // this is about the sweep's own rule, not about how fast one request follows another: the one
    // meant to expire is created below.
    let living = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout_sec":3600}"#,
    )
    .await;
    assert_eq!(living.status, 201);
    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);
    assert_eq!(runner.sandboxes().live(), 1);

    // One whose time is up is swept, and the sweep journals its expiry.
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout_sec":1}"#,
    )
    .await;
    assert_eq!(created.status, 201);

    reap_until_destroyed(&runner, &engine).await;

    assert_eq!(engine.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(
        runner.sandboxes().live(),
        1,
        "only the sandbox whose time was up is swept"
    );
    // The create's own paperwork is the transport's job; the sweep journals its expiry.
    wait_for(runner.journal(), 1);
    let journal = std::fs::read_to_string(runner.journal().path()).expect("journal");
    let line: serde_json::Value =
        serde_json::from_str(journal.lines().next().expect("a line")).expect("JSON");
    assert_eq!(line["kind"], "expire");
    assert_eq!(line["reason"], "timeout");
    assert_eq!(line["key_id"], "k-1");
    let gone = call(
        &runner,
        http::Method::DELETE,
        &format!("/api/v1/sandboxes/{}", created_id(&created)),
        "",
    )
    .await;
    assert_eq!(gone.status, 200, "a reaped sandbox reads as destroyed");
}

#[tokio::test]
async fn tenant_cap_and_project_scope_are_enforced_before_the_facade() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#"["p-1"]"#, "1");

    let other_project = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"project_id":"p-2"}"#,
    )
    .await;
    assert_eq!(other_project.status, 403);
    assert_eq!(other_project.body, br#"{"error":"forbidden"}"#.to_vec());

    let first = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"project_id":"p-1"}"#,
    )
    .await;
    assert_eq!(first.status, 201);
    let second = call(&runner, http::Method::POST, "/api/v1/sandboxes", "").await;
    assert_eq!(second.status, 429);
}
