use std::sync::{Arc, Mutex};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{
    BackendTarget, ExecutionReceipt, InspectResult, InstanceId, MachineState, RuntimeFailure,
    RuntimeRequest, RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const INSTANCE_ID: &str = "22222222222222222222222222222222";
const OPERATION_ID: &str = "33333333333333333333333333333333";

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
        Ok(RuntimeResponse::Inspect(InspectResult::new(
            InstanceId::new(INSTANCE_ID).expect("instance ID"),
            MachineState::Ready,
            BackendTarget::Macos,
            Some(
                ExecutionReceipt::new(serde_json::json!({
                    "schema_version": 1,
                    "evidence_class": "basic"
                }))
                .expect("bounded receipt"),
            ),
        )))
    }
}

#[tokio::test]
async fn inspect_is_bounded_and_preserves_the_latest_receipt() {
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
        "backend": "macos"
    })
    .as_object()
    .cloned()
    .expect("object arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_inspect").with_arguments(arguments))
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["operation"], "inspect");
    assert_eq!(structured["operation_id"], OPERATION_ID);
    assert_eq!(structured["result"]["instance_id"], INSTANCE_ID);
    assert_eq!(structured["result"]["state"], "ready");
    assert_eq!(structured["result"]["backend"], "macos");
    assert_eq!(structured["receipt"]["evidence_class"], "basic");

    {
        let requests = requests.lock().expect("request ledger");
        let RuntimeRequest::Inspect(request) = requests.first().expect("inspect request") else {
            panic!("expected inspect request");
        };
        assert_eq!(request.instance_id().as_str(), INSTANCE_ID);
        assert_eq!(request.operation_id().as_str(), OPERATION_ID);
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
