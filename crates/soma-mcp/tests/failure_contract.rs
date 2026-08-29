use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{
    CommandStatus, ExecutionReceipt, RuntimeFailure, RuntimeFailureKind, RuntimeRequest,
    RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const OPERATION_ID: &str = "11111111111111111111111111111111";
const INSTANCE_ID: &str = "22222222222222222222222222222222";
const SECRET_ARGUMENT: &str = "secret-token-must-not-echo";

#[derive(Clone, Copy)]
struct FailingRuntime;

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the synchronous test double implements the asynchronous runtime seam"
)]
impl ToolRuntime for FailingRuntime {
    async fn invoke(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        Err(RuntimeFailure::new(RuntimeFailureKind::Internal))
    }
}

#[derive(Clone, Copy)]
struct EvidenceFailureRuntime;

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the synchronous test double implements the asynchronous runtime seam"
)]
impl ToolRuntime for EvidenceFailureRuntime {
    async fn invoke(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        Err(RuntimeFailure::with_command_evidence(
            RuntimeFailureKind::Timeout,
            ExecutionReceipt::new(serde_json::json!({
                "schema_version": 1,
                "operation_id": OPERATION_ID,
                "terminal_status": "timed_out"
            }))
            .expect("bounded receipt"),
            CommandStatus::TimedOut,
            vec![0xff, b'o'],
            vec![0xfe],
        ))
    }
}

#[tokio::test]
async fn bounded_failure_envelope_returns_retry_ids_without_echoing_input() {
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(FailingRuntime)
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = ().serve(client_transport).await.expect("client connects");
    let arguments = serde_json::json!({
        "operation_id": OPERATION_ID,
        "instance_id": INSTANCE_ID,
        "image": "ubuntu:24.04",
        "executable": "/bin/echo",
        "arguments": [SECRET_ARGUMENT]
    })
    .as_object()
    .cloned()
    .expect("object arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_run").with_arguments(arguments))
        .await
        .expect("tool failure result");

    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.expect("structured failure");
    assert_eq!(structured["schema"], "soma.mcp.v1");
    assert_eq!(structured["operation"], "run");
    assert_eq!(structured["operation_id"], OPERATION_ID);
    assert_eq!(structured["error"]["instance_id"], INSTANCE_ID);
    assert_eq!(structured["error"]["code"], "internal");
    assert_eq!(structured["receipt"], serde_json::Value::Null);
    assert!(!structured.to_string().contains(SECRET_ARGUMENT));

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}

#[tokio::test]
async fn command_failure_preserves_bounded_binary_output_and_validated_receipt() {
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(EvidenceFailureRuntime)
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = ().serve(client_transport).await.expect("client connects");
    let arguments = serde_json::json!({
        "operation_id": OPERATION_ID,
        "instance_id": INSTANCE_ID,
        "image": "ubuntu:24.04",
        "executable": "/bin/echo",
        "arguments": ["safe"]
    })
    .as_object()
    .cloned()
    .expect("object arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_run").with_arguments(arguments))
        .await
        .expect("tool failure result");

    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.expect("structured failure");
    assert_eq!(structured["error"]["code"], "timeout");
    assert_eq!(structured["result"]["instance_id"], INSTANCE_ID);
    assert_eq!(structured["result"]["status"]["kind"], "timed_out");
    assert_eq!(structured["result"]["stdout"]["encoding"], "base64");
    assert_eq!(structured["result"]["stdout"]["byte_length"], 2);
    assert_eq!(structured["result"]["stdout"]["data"], "/28=");
    assert_eq!(structured["result"]["stderr"]["data"], "/g==");
    assert_eq!(structured["receipt"]["schema_version"], 1);
    assert_eq!(structured["receipt"]["operation_id"], OPERATION_ID);

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
