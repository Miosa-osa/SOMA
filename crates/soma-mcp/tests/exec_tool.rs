use std::sync::{Arc, Mutex};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{
    CommandResult, CommandStatus, ExecutionReceipt, InstanceId, RuntimeFailure, RuntimeRequest,
    RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const OPERATION_ID: &str = "33333333333333333333333333333333";
const INSTANCE_ID: &str = "22222222222222222222222222222222";

#[derive(Clone, Default)]
struct FakeRuntime {
    requests: Arc<Mutex<Vec<RuntimeRequest>>>,
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the synchronous test double implements the asynchronous runtime seam"
)]
impl ToolRuntime for FakeRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        self.requests.lock().expect("request ledger").push(request);
        Ok(RuntimeResponse::Exec(CommandResult::new(
            InstanceId::new(INSTANCE_ID).expect("instance ID"),
            CommandStatus::Signaled { signal: Some(15) },
            vec![b'o', b'k'],
            vec![0xff],
            ExecutionReceipt::new(serde_json::json!({
                "schema_version": 1,
                "terminal_status": { "signaled": 15 }
            }))
            .expect("bounded receipt"),
        )))
    }
}

#[tokio::test]
async fn exec_preserves_direct_argv_and_binary_terminal_output() {
    let runtime = FakeRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(runtime)
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
        "executable": "/usr/bin/printf",
        "arguments": ["%s", "hello world"],
        "timeout_ms": 1000,
        "max_output_bytes": 32,
        "backend": "macos"
    })
    .as_object()
    .cloned()
    .expect("object arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_exec").with_arguments(arguments))
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["operation"], "exec");
    assert_eq!(structured["operation_id"], OPERATION_ID);
    assert_eq!(structured["result"]["status"]["kind"], "signaled");
    assert_eq!(structured["result"]["status"]["signal"], 15);
    assert_eq!(structured["result"]["stderr"]["data"], "/w==");

    {
        let requests = requests.lock().expect("request ledger");
        let RuntimeRequest::Exec(request) = requests.first().expect("exec request") else {
            panic!("expected exec request");
        };
        assert_eq!(request.operation_id().as_str(), OPERATION_ID);
        assert_eq!(request.instance_id().as_str(), INSTANCE_ID);
        assert_eq!(request.command().executable(), "/usr/bin/printf");
        assert_eq!(request.command().arguments(), ["%s", "hello world"]);
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
