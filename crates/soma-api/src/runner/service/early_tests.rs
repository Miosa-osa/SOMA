//! The two opt-in create fields: an answer before the sandbox is up, and a command in the create.
//!
//! Both go through the whole service with the fake facade the other flow tests use, so what is
//! asserted here is the runner's own behaviour: what each answer says, what a later call does
//! when the launch it names is still in flight, and what a call reads when that launch failed.

use std::{
    sync::{
        Arc,
        atomic::Ordering,
        mpsc,
    },
    time::Duration,
};

use soma::{BackendFailureKind, TerminalStatus};

use super::{
    RunnerResponse,
    flow_tests::{Engine, body_of, call, created_id, runner, timing_of},
};
use crate::runner::{keys::tests::TENANT, public_wire};

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
    assert_eq!(runner.sandboxes().live(), 1, "the sandbox exists and is held");

    // The launch had not finished when this answer was formed, so this request spent no time in
    // the launch's own phases and its `exec` is zero: the call it names had not returned.
    let timing = timing_of(&created);
    assert_eq!(timing["exec"], 0);
    assert!(timing["ready"] == 0 && timing["assign"] == 0, "{timing:?}");
    assert_eq!(json["create_ms"].as_u64().expect("create_ms"), timing["pool"] / 1_000);
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
async fn a_create_and_run_answers_the_create_and_its_command_together() {
    let engine = Arc::new(Engine::default());
    *engine.exec_status.lock().expect("status") = Some(TerminalStatus::Exited { code: 0 });
    let runner = runner(&engine, r#""*""#, "null");

    let created = create(&runner,
        r#"{"run":{"command":"node -v","timeout_ms":5000}}"#,
    )
    .await;
    assert_eq!(created.status, 201);
    let json = body_of(&created);
    assert_eq!(json["state"], "running", "a create-and-run waits for its sandbox");
    assert_eq!(json["run"]["exit_code"], 0);
    assert!(json["run"]["duration_ms"].is_u64());
    assert_eq!(json["run"]["stdout"], "");
    assert_eq!(json["run"]["stderr"], "no");
    assert!(
        json["run"]["error"].is_null(),
        "the command ended, so it carries no error: {}",
        created.body.len()
    );
    assert_eq!(engine.execs.load(Ordering::SeqCst), 1);

    // The command ran on the create's own facade call, so the `run` segment is a part of `exec`
    // rather than a segment beside it, and create_ms is still the pool and exec this answer
    // reports.
    let timing = timing_of(&created);
    assert!(timing.contains_key("run"), "{timing:?}");
    assert!(timing["run"] <= timing["exec"], "{timing:?}");
    assert!(
        !timing.contains_key("wait"),
        "a create-and-run waits, so it never reports a wait of its own: {timing:?}"
    );
    assert_eq!(
        json["create_ms"].as_u64().expect("create_ms"),
        (timing["pool"] + timing["exec"]) / 1_000
    );
}

#[tokio::test]
async fn a_create_and_run_reports_a_command_that_exited_non_zero_exactly_as_exec_does() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");

    // The double's commands exit 3 with `no` on stderr, which is a result rather than a failure.
    let created = create(&runner, r#"{"run":{"command":"false"}}"#).await;
    assert_eq!(created.status, 201, "a failing command does not fail the create");
    let json = body_of(&created);
    assert_eq!(json["run"]["exit_code"], 3);
    assert!(json["run"]["error"].is_null());
    assert_eq!(json["run"]["stderr"], "no");

    // The exec endpoint answers the same, and the sandbox is there for it to answer about.
    let id = created_id(&created);
    let executed = exec(&runner, &id, r#"{"command":"false"}"#).await;
    assert_eq!(executed.status, 200);
    let executed = body_of(&executed);
    assert_eq!(json["run"]["exit_code"], executed["data"]["exit_code"]);
    assert_eq!(json["run"]["stdout"], executed["data"]["stdout"]);
    assert_eq!(json["run"]["stderr"], executed["data"]["stderr"]);
}

#[tokio::test]
async fn a_create_and_run_reports_a_command_that_produced_no_result_with_the_exec_error() {
    let engine = Arc::new(Engine::default());
    // A command that produced no result of its own: the engine reports the outcome rather than an
    // exit code, which the exec endpoint answers as the agent being unavailable.
    *engine.exec_status.lock().expect("status") = Some(TerminalStatus::TimedOut);
    let runner = runner(&engine, r#""*""#, "null");

    let created = create(&runner, r#"{"run":{"command":"sleep 60","timeout_ms":100}}"#).await;
    assert_eq!(created.status, 201, "the sandbox still exists and the create still succeeds");
    let json = body_of(&created);
    assert!(
        json["run"]["exit_code"].is_null(),
        "there was no result to report: {}",
        String::from_utf8_lossy(&created.body)
    );

    *engine.exec_status.lock().expect("status") = Some(TerminalStatus::TimedOut);
    let id = created_id(&created);
    let executed = exec(&runner, &id, r#"{"command":"sleep 60","timeout_ms":100}"#).await;
    let executed = body_of(&executed);
    assert_eq!(
        json["run"]["error"], executed["error"],
        "the same outcome is answered the same way by both doors"
    );
    assert_eq!(executed["error"]["code"], "AGENT_UNAVAILABLE");
}

#[tokio::test]
async fn a_command_the_exec_door_would_refuse_refuses_the_whole_create() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");

    for (body, code) in [
        (r#"{"run":{"command":"node -v","timeout_ms":0}}"#, "INVALID_TIMEOUT"),
        (r#"{"run":{"command":"node -v","extra":true}}"#, "INVALID_PARAM"),
        (r#"{"run":{"command":""}}"#, "MISSING_PARAM"),
        (r#"{"run":"node -v"}"#, "INVALID_PARAM"),
    ] {
        let refused = create(&runner, body).await;
        assert_eq!(refused.status, 400, "{body}: {:?}", refused.body);
        assert_eq!(body_of(&refused)["error"]["code"], code, "{body}");
        assert!(
            refused.journal.is_some(),
            "{body}: a refused create is paperwork"
        );
    }
    assert_eq!(
        engine.launches.load(Ordering::SeqCst),
        0,
        "a create whose command the runtime would refuse never launches"
    );
    assert_eq!(engine.execs.load(Ordering::SeqCst), 0);

    // A `run` block that is null is a create with no command, exactly as an absent one is; a
    // block that is present but names no command is refused.
    let bare = create(&runner, r#"{"run":null}"#).await;
    assert_eq!(bare.status, 201);
    assert!(body_of(&bare).get("run").is_none());
    let empty = create(&runner, r#"{"run":{}}"#).await;
    assert_eq!(empty.status, 400);
    assert_eq!(body_of(&empty)["error"]["code"], "MISSING_PARAM");
    assert_eq!(engine.launches.load(Ordering::SeqCst), 1, "only the bare one launched");
}

#[tokio::test]
async fn a_create_that_runs_a_command_may_not_also_ask_to_answer_early() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");

    let refused = create(
        &runner,
        r#"{"run":{"command":"node -v"},"wait_ready":false}"#,
    )
    .await;
    assert_eq!(refused.status, 400);
    let json = body_of(&refused);
    assert_eq!(json["error"]["code"], "INVALID_PARAM");
    assert_eq!(json["error"]["details"]["field"], "wait_ready");
    assert_eq!(engine.launches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_create_field_the_platform_does_not_define_is_still_refused_by_name() {
    // The two new fields are the create vocabulary's, not a hole in it.
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");

    let refused = create(&runner, r#"{"wait_readyy":false}"#).await;
    assert_eq!(refused.status, 400);
    assert_eq!(body_of(&refused)["error"]["details"]["field"], "wait_readyy");
}
