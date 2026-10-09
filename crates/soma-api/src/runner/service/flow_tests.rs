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
    BackendKind, DestroyMachineRequest, ExecuteMachineRequest, ExecutionReceipt,
    FileMachineRequest, InspectMachineRequest, LaunchMachineRequest, MachineName, ManagedFailure,
    PtyMachineRequest, SandboxEntry, SandboxLiveness, SandboxPhase, StopMachineRequest,
    TerminalStatus,
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

const RECEIPT: &str = include_str!("../../../tests/fixtures/receipt.json");

#[derive(Default)]
pub(super) struct Engine {
    pub(super) destroys: AtomicUsize,
    pub(super) listed: Mutex<Vec<SandboxEntry>>,
    /// How long each command takes, for tests of commands that outlast the idle timeout.
    pub(super) exec_delay: Mutex<Duration>,
    /// How long each terminal call takes, as a long `read` with `wait_ms` would.
    pub(super) terminal_delay: Mutex<Duration>,
}

struct Fake(Arc<Engine>);

fn lifecycle() -> LifecycleOutcome {
    let receipt: ExecutionReceipt = serde_json::from_str(RECEIPT).expect("receipt");
    LifecycleOutcome {
        instance_id: receipt.instance_id().clone(),
        receipt,
    }
}

impl SandboxFacade for Fake {
    fn hosts_addressable_sandboxes(&self) -> bool {
        true
    }

    fn launch(&mut self, _: LaunchMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        Ok(lifecycle())
    }

    fn inspect(&mut self, _: InspectMachineRequest) -> Result<SandboxSnapshot, ManagedFailure> {
        unreachable!("not a runner route")
    }

    fn execute(&mut self, _: ExecuteMachineRequest) -> Result<CommandOutcome, ManagedFailure> {
        std::thread::sleep(*self.0.exec_delay.lock().expect("delay"));
        let outcome = lifecycle();
        Ok(CommandOutcome {
            instance_id: outcome.instance_id,
            status: TerminalStatus::Exited { code: 3 },
            stdout: Vec::new(),
            stderr: b"no".to_vec(),
            receipt: outcome.receipt,
        })
    }

    fn file(&mut self, _: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        unreachable!("not a runner route")
    }

    fn terminal(&mut self, _: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        std::thread::sleep(*self.0.terminal_delay.lock().expect("delay"));
        Ok(TerminalOutcome {
            instance_id: lifecycle().instance_id,
            operation: "read",
            answer: soma::PtyAnswer::Output {
                bytes: b"$ ".to_vec(),
                end: false,
            },
        })
    }

    fn list(&mut self) -> Result<Vec<SandboxEntry>, ManagedFailure> {
        Ok(self.0.listed.lock().expect("listed").clone())
    }

    fn stop(&mut self, _: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        unreachable!("not a runner route")
    }

    fn destroy(&mut self, _: DestroyMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.0.destroys.fetch_add(1, Ordering::SeqCst);
        Ok(lifecycle())
    }
}

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
        })
        .await
}

fn created_id(response: &RunnerResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON");
    body["id"].as_str().expect("id").to_owned()
}

/// Sweeps until the expired sandbox is gone, or fails once the deadline passes.
///
/// A sweep compares wall-clock instants and every destroy goes back through the facade, so on a
/// loaded machine one sweep can need longer than the timeout itself to notice an expiry. Waiting
/// on the condition rather than on a fixed margin is what keeps these tests about expiry instead
/// of about how busy the host running them is.
pub(super) async fn reap_until_destroyed(runner: &Runner, engine: &Engine) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.destroys.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "the reaper never destroyed a sandbox whose lifetime had run out"
        );
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
    // Created with a lifetime no load can make this test outlast, so the sweep below proves what
    // the contract says it proves, that a sandbox inside its lifetime survives one, instead of
    // racing the clock: at one second, a create and a sweep on a loaded machine could take longer
    // than the lifetime and the sandbox was already expired before it was ever swept.
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout_sec":600}"#,
    )
    .await;
    assert_eq!(created.status, 201);

    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);

    // Shortened to a second, and then destroyed by the sweep that follows the expiry.
    let shortened = call(
        &runner,
        http::Method::PATCH,
        &format!("/api/v1/sandboxes/{}", created_id(&created)),
        r#"{"timeout":1}"#,
    )
    .await;
    assert_eq!(shortened.status, 200);

    reap_until_destroyed(&runner, &engine).await;

    assert_eq!(engine.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(runner.sandboxes().live(), 0);
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
