//! The other half of the two create fields: the command a create may carry.
//!
//! It reads its parent's fixtures, so the two files are one test suite split where the create
//! that answers early ends and the create that runs a command begins.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's fixtures moved wholesale; naming every import would obscure it"
)]
use super::*;

#[tokio::test]
async fn a_create_and_run_answers_the_create_and_its_command_together() {
    let engine = Arc::new(Engine::default());
    *engine.exec_status.lock().expect("status") = Some(TerminalStatus::Exited { code: 0 });
    let runner = runner(&engine, r#""*""#, "null");

    let created = create(
        &runner,
        r#"{"run":{"command":"node -v","timeout_ms":5000}}"#,
    )
    .await;
    assert_eq!(created.status, 201);
    let json = body_of(&created);
    assert_eq!(
        json["state"], "running",
        "a create-and-run waits for its sandbox"
    );
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
    assert_eq!(
        created.status, 201,
        "a failing command does not fail the create"
    );
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

    let created = create(
        &runner,
        r#"{"run":{"command":"sleep 60","timeout_ms":100}}"#,
    )
    .await;
    assert_eq!(
        created.status, 201,
        "the sandbox still exists and the create still succeeds"
    );
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
        (
            r#"{"run":{"command":"node -v","timeout_ms":0}}"#,
            "INVALID_TIMEOUT",
        ),
        (
            r#"{"run":{"command":"node -v","extra":true}}"#,
            "INVALID_PARAM",
        ),
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
    assert_eq!(
        engine.launches.load(Ordering::SeqCst),
        1,
        "only the bare one launched"
    );
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
    assert_eq!(
        body_of(&refused)["error"]["details"]["field"],
        "wait_readyy"
    );
}
