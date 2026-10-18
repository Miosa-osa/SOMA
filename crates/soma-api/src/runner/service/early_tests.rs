//! The two opt-in create fields: an answer before the sandbox is up, and a command in the create.
//!
//! Both go through the whole service with the fake facade the other flow tests use, so what is
//! asserted here is the runner's own behaviour: what each answer says, what a later call does
//! when the launch it names is still in flight, and what a call reads when that launch failed.

use std::{
    sync::{Arc, atomic::Ordering, mpsc},
    time::Duration,
};

use soma::{BackendFailureKind, TerminalStatus};

use super::{
    RunnerResponse,
    flow_tests::{Engine, body_of, call, created_id, runner, timing_of},
};
use crate::runner::{keys::tests::TENANT, public_wire};

mod run;

/// One create, with the body a test names.
async fn create(runner: &super::Runner, body: &str) -> RunnerResponse {
    call(runner, http::Method::POST, "/api/v1/sandboxes", body).await
}

/// One command, on the sandbox `id` names.
async fn exec(runner: &super::Runner, id: &str, body: &str) -> RunnerResponse {
    call(
        runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        body,
    )
    .await
}

/// The tenant's live sandbox count on this runner, which every end of a sandbox gives back.
fn slots(runner: &super::Runner) -> i64 {
    runner.keys().counter(TENANT).load(Ordering::Acquire)
}

#[tokio::test]
async fn a_create_that_did_not_ask_for_an_early_answer_is_unchanged() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");

    for body in ["", r#"{"wait_ready":true}"#, r#"{"wait_ready":"1"}"#] {
        let created = create(&runner, body).await;
        assert_eq!(created.status, 201, "{body}: {}", created.status);
        let json = body_of(&created);
        assert_eq!(json["state"], "running", "{body}");
        assert!(
            json.get("run").is_none(),
            "a create with no command carries no `run` key: {body}"
        );

        // The whole header of a synchronous create is the ten segments it always had, and
        // `create_ms` is the `pool` + `exec` those same segments report (T4).
        let timing = timing_of(&created);
        assert_eq!(
            timing.len(),
            10,
            "{body}: an early answer is the only answer that adds a segment: {timing:?}"
        );
        assert_eq!(
            json["create_ms"].as_u64().expect("create_ms"),
            (timing["pool"] + timing["exec"]) / 1_000,
            "{body}: create_ms is the pool and exec this answer reports"
        );
    }
    assert_eq!(engine.launches.load(Ordering::SeqCst), 3);
    assert_eq!(runner.sandboxes().live(), 3);
    assert_eq!(slots(&runner), 3);
}

#[tokio::test]
async fn an_early_answer_reports_starting_and_a_later_call_waits_for_the_launch() {
    let engine = Arc::new(Engine::default());
    // The launch holds until this test releases it, which is what keeps it in flight while the
    // create answers and the command arrives.
    let (release, gate) = mpsc::channel();
    *engine.launch_gate.lock().expect("gate") = Some(gate);
    let runner = runner(&engine, r#""*""#, "null");

    let created = create(&runner, r#"{"wait_ready":false}"#).await;
    assert_eq!(created.status, 201);
    let json = body_of(&created);
    let id = created_id(&created);
    assert_eq!(json["state"], "starting");
    assert_eq!(engine.launches.load(Ordering::SeqCst), 1);
    assert_eq!(
        runner.sandboxes().live(),
        1,
        "the sandbox exists and is held"
    );

    // The launch had not finished when this answer was formed, so this request spent no time in
    // the launch's own phases and its `exec` is zero: the call it names had not returned.
    let timing = timing_of(&created);
    assert_eq!(timing["exec"], 0);
    assert!(timing["ready"] == 0 && timing["assign"] == 0, "{timing:?}");
    assert_eq!(
        json["create_ms"].as_u64().expect("create_ms"),
        timing["pool"] / 1_000
    );
    assert!(
        !timing.contains_key("wait") && !timing.contains_key("run"),
        "only a call that waited, or ran a command, carries those: {timing:?}"
    );

    // A command on that sandbox waits for the launch instead of being refused for a machine that
    // is still coming up.
    let mut command = Box::pin(exec(&runner, &id, r#"{"command":"false"}"#));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut command)
            .await
            .is_err(),
        "the command must wait for the launch"
    );
    release.send(()).expect("the launch is released");
    let answered = command.await;
    assert_eq!(answered.status, 200);
    assert_eq!(
        answered.body,
        br#"{"data":{"exit_code":3,"stderr":"no","stdout":""}}"#.to_vec()
    );
    let timing = timing_of(&answered);
    assert!(
        timing["wait"] > 0,
        "the command reports how long it waited: {timing:?}"
    );
    assert_eq!(runner.sandboxes().live(), 1);
}

#[tokio::test]
async fn a_launch_that_failed_after_an_early_answer_answers_the_next_call_with_the_creates_failure()
{
    let engine = Arc::new(Engine::default());
    let (release, gate) = mpsc::channel();
    *engine.launch_gate.lock().expect("gate") = Some(gate);
    *engine.launch_refusal.lock().expect("refusal") = Some(BackendFailureKind::Unavailable);
    let runner = runner(&engine, r#""*""#, "null");

    let created = create(&runner, r#"{"wait_ready":false}"#).await;
    assert_eq!(created.status, 201, "the answer is this create's own");
    let id = created_id(&created);
    assert_eq!(body_of(&created)["state"], "starting");
    assert_eq!(slots(&runner), 1);
    release.send(()).expect("the launch runs and fails");

    // The next call reads what the synchronous create would have answered: the same status, the
    // same body, and the same retry hint.
    let failed = exec(&runner, &id, r#"{"command":"false"}"#).await;
    assert_eq!(failed.status, 503);
    assert_eq!(failed.body, public_wire::create_unavailable());
    assert_eq!(
        failed
            .headers
            .iter()
            .find(|(name, _)| *name == "retry-after")
            .map(|(_, value)| value.as_str()),
        Some("1")
    );
    assert!(failed.journal.is_some(), "the call is paperwork");

    // The sandbox is gone: it holds no slot on its tenant, and it is not one this host holds. The
    // record stays as a tombstone for as long as a destroyed one does, so a call that arrives
    // after the failure still reads the create's answer rather than a `404` that hides it.
    assert_eq!(slots(&runner), 0);
    assert_eq!(runner.sandboxes().live(), 0);
    let again = exec(&runner, &id, r#"{"command":"false"}"#).await;
    assert_eq!(again.status, 503);
    assert_eq!(again.body, public_wire::create_unavailable());
}

#[tokio::test]
async fn an_abandoned_create_gives_its_tenant_slot_back() {
    let engine = Arc::new(Engine::default());
    // The launch holds until this test releases it, so the request can be dropped while the
    // launch it started is still in flight - which is exactly what a client that resets the
    // stream does to the handler it was waiting on.
    let (release, gate) = mpsc::channel();
    *engine.launch_gate.lock().expect("gate") = Some(gate);
    // One slot for this tenant, so a slot that is not given back is a create that cannot happen.
    let runner = runner(&engine, r#""*""#, "1");

    {
        let mut abandoned = Box::pin(create(&runner, "{}"));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut abandoned)
                .await
                .is_err(),
            "the create must still be waiting on its launch"
        );
        // Dropping the future here is the client's reset: nothing will settle this record.
    }

    // The slot comes back as the abandoned request is dropped, so this waits for it rather than
    // reading at one exact instant. What the test is about is a slot that is *never* given back,
    // and that one does not arrive however long this waits.
    for _ in 0..20 {
        if slots(&runner) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        slots(&runner),
        0,
        "an abandoned create must give its tenant slot back"
    );

    let _ = release.send(());
    let after = create(&runner, "{}").await;
    assert_eq!(
        after.status, 201,
        "the slot must be free again for the next create"
    );
}

#[tokio::test]
async fn an_abandoned_create_takes_down_the_machine_it_was_building() {
    let engine = Arc::new(Engine::default());
    let (release, gate) = mpsc::channel();
    *engine.launch_gate.lock().expect("gate") = Some(gate);
    let runner = runner(&engine, r#""*""#, "1");

    {
        let mut abandoned = Box::pin(create(&runner, "{}"));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut abandoned)
                .await
                .is_err(),
            "the create must still be waiting on its launch"
        );
        // The launch itself cannot be cancelled, so it is released here rather than dropped:
        // finishing is what makes it find that the record its machine belongs to has gone.
    }
    let _ = release.send(());

    for _ in 0..40 {
        if engine.destroys.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        engine.destroys.load(Ordering::SeqCst),
        1,
        "a machine whose create was abandoned must not be left running"
    );
    assert_eq!(slots(&runner), 0, "and the tenant slot must be back");
}
