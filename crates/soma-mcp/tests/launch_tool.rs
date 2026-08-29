use std::sync::{Arc, Mutex};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma::{DnsPolicy, EgressPolicy};
use soma_mcp::{
    ExecutionReceipt, InstanceId, MachineResult, MachineState, RuntimeFailure, RuntimeRequest,
    RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const OPERATION_ID: &str = "11111111111111111111111111111111";
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
        Ok(RuntimeResponse::Launch(MachineResult::new(
            InstanceId::new(INSTANCE_ID).expect("instance ID"),
            MachineState::Ready,
            ExecutionReceipt::new(serde_json::json!({
                "schema_version": 1,
                "terminal_status": "ready"
            }))
            .expect("bounded receipt"),
        )))
    }
}

#[tokio::test]
async fn launch_returns_the_managed_instance_and_receipt() {
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
        "image": "ubuntu:24.04",
        "display_name": "agent-workspace",
        "vcpu_count": 1,
        "memory_mib": 1024,
        "storage_mib": 4096,
        "network": {"egress": "unrestricted", "dns": "system"},
        "backend": "macos"
    })
    .as_object()
    .cloned()
    .expect("object arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_launch").with_arguments(arguments))
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["operation"], "launch");
    assert_eq!(structured["operation_id"], OPERATION_ID);
    assert_eq!(structured["result"]["instance_id"], INSTANCE_ID);
    assert_eq!(structured["result"]["state"], "ready");
    assert_eq!(structured["receipt"]["terminal_status"], "ready");

    {
        let requests = requests.lock().expect("request ledger");
        let RuntimeRequest::Launch(request) = requests.first().expect("launch request") else {
            panic!("expected launch request");
        };
        assert_eq!(request.operation_id().as_str(), OPERATION_ID);
        assert_eq!(request.instance_id().as_str(), INSTANCE_ID);
        assert_eq!(request.image().as_str(), "ubuntu:24.04");
        assert_eq!(
            request.display_name().map(soma_mcp::DisplayName::as_str),
            Some("agent-workspace")
        );
        assert_eq!(request.shape().storage_mib(), 4096);
        let network = request.shape().capabilities().network_policy();
        assert_eq!(network.egress(), EgressPolicy::Unrestricted);
        assert_eq!(network.dns(), &DnsPolicy::System);
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
