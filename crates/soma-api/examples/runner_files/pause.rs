//! Runs pause through the same authenticated runner routes as the file proof.
use http::Method;
use soma_api::runner::Runner;

use super::{
    flow::send,
    harness::{expect, json, request},
};

pub(super) async fn run(runner: &Runner, id: &str) -> Result<(), String> {
    let sandbox = format!("/api/v1/sandboxes/{id}");
    let pause = format!("{sandbox}/pause");
    let first = send(runner, "pause", request(Method::POST, &pause, "{}")).await;
    expect(&first, 200)?;
    let repeat = send(runner, "repeat pause", request(Method::POST, &pause, "")).await;
    expect(&repeat, 200)?;
    let inspected = send(
        runner,
        "inspect paused sandbox",
        request(Method::GET, &sandbox, ""),
    )
    .await;
    expect(&inspected, 200)?;
    for answer in [&first, &repeat, &inspected] {
        if json(answer)["result"]["state"] != "paused" {
            return Err(format!("expected paused state, got {}", json(answer)));
        }
    }
    let command = request(
        Method::POST,
        &format!("{sandbox}/exec"),
        r#"{"command":"echo unexpected"}"#,
    );
    let refused = send(runner, "exec while paused", command).await;
    expect(&refused, 409)?;
    if json(&refused)["error"]["code"] != "SANDBOX_PAUSED" {
        return Err(format!("expected pause refusal, got {}", json(&refused)));
    }
    expect(
        &send(
            runner,
            "inspect after refusal",
            request(Method::GET, &sandbox, ""),
        )
        .await,
        200,
    )?;
    Ok(())
}
