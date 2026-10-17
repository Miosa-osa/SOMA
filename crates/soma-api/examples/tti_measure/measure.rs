//! The measurement itself: one create per sample, and the three paths side by side.

use std::sync::Arc;

use soma_api::runner::Runner;

use super::{
    harness::request,
    report::{Timing, answer_body, elapsed_us, median, optional, report},
};

/// The command every flow runs: the one a time-to-interactive benchmark runs first.
pub(super) const COMMAND: &str = "node -v";
/// How many creates each measurement takes at one concurrency.
pub(super) const ROUNDS_AT_ONE: usize = 30;
pub(super) const ROUNDS_AT_CONCURRENCY: usize = 2;
pub(super) const CONCURRENT: usize = 32;

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

/// Creates one sandbox with `body`, runs the command the flow asks for, and destroys it.
async fn one_create(runner: &Runner, body: &str, second_call: bool) -> Measured {
    let started = std::time::Instant::now();
    let created = runner
        .handle(request(http::Method::POST, "/api/v1/sandboxes", body))
        .await;
    assert_eq!(
        created.status,
        201,
        "create: {}",
        String::from_utf8_lossy(&created.body)
    );
    let create_us = elapsed_us(started);
    let create = Timing::of(&created);
    let id = answer_body(&created)["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let mut command_us = None;
    let (command, total_us) = if second_call {
        let started = std::time::Instant::now();
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
        (Some(Timing::of(&executed)), create_us + elapsed_us(started))
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

/// Every flow at every concurrency, printed as one table.
pub(super) async fn all(runner: &Arc<Runner>) {
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
                    let runner = Arc::clone(runner);
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
                    if let Some(wait) = measured.command.as_ref().and_then(|timing| timing.wait_us)
                    {
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
