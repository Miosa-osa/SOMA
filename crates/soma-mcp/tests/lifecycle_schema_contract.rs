use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{RuntimeFailure, RuntimeRequest, RuntimeResponse, SomaMcpServer, ToolRuntime};

const INSTANCE_ID: &str = "22222222222222222222222222222222";

#[derive(Clone, Copy)]
struct PanicRuntime;

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the test double proves rejected input never reaches the runtime seam"
)]
impl ToolRuntime for PanicRuntime {
    async fn invoke(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        panic!("unknown lifecycle fields must fail before runtime admission")
    }
}

#[tokio::test]
async fn lifecycle_tools_reject_an_unimplemented_caller_timeout() {
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(PanicRuntime)
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = ().serve(client_transport).await.expect("client connects");

    for (tool, arguments) in [
        (
            "soma_launch",
            serde_json::json!({
                "instance_id": INSTANCE_ID,
                "image": "ubuntu:24.04",
                "timeout_ms": 1
            }),
        ),
        (
            "soma_inspect",
            serde_json::json!({
                "instance_id": INSTANCE_ID,
                "timeout_ms": 1
            }),
        ),
        (
            "soma_stop",
            serde_json::json!({
                "instance_id": INSTANCE_ID,
                "timeout_ms": 1
            }),
        ),
        (
            "soma_destroy",
            serde_json::json!({
                "instance_id": INSTANCE_ID,
                "timeout_ms": 1
            }),
        ),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new(tool)
                    .with_arguments(arguments.as_object().cloned().expect("object arguments")),
            )
            .await;
        let rejected = result
            .as_ref()
            .is_ok_and(|response| response.is_error == Some(true))
            || result.is_err();
        assert!(rejected, "{tool} accepted an unimplemented timeout");
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
