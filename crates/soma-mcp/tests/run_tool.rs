use std::sync::{Arc, Mutex};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma::{DnsPolicy, EgressPolicy};
use soma_mcp::{
    CommandResult, CommandStatus, ExecutionReceipt, InstanceId, RuntimeFailure, RuntimeRequest,
    RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const INSTANCE_ID: &str = "22222222222222222222222222222222";
const OPERATION_ID: &str = "11111111111111111111111111111111";

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
        Ok(RuntimeResponse::Run(CommandResult::new(
            InstanceId::new(INSTANCE_ID).expect("instance ID"),
            CommandStatus::Exited { code: 0 },
            vec![0xff, 0x00, b'o', b'k'],
            vec![0xfe],
            ExecutionReceipt::new(serde_json::json!({
                "schema_version": 1,
                "operation_id": "11111111111111111111111111111111",
                "requested_shape": {
                    "vcpu_count": 2,
                    "memory_mib": 2048,
                    "storage_mib": 8192
                }
            }))
            .expect("bounded receipt"),
        )))
    }
}

#[tokio::test]
async fn run_preserves_argv_and_returns_binary_output_with_the_receipt() {
    let runtime = FakeRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (server_transport, client_transport) = tokio::io::duplex(256 * 1024);
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
        "image": "node:22",
        "display_name": "node-evaluator",
        "executable": "/usr/local/bin/node",
        "arguments": ["--eval", "process.stdout.write('ok')"],
        "vcpu_count": 2,
        "memory_mib": 2048,
        "storage_mib": 8192,
        "network": {"egress": "denied", "dns": "denied"},
        "timeout_ms": 5000,
        "max_output_bytes": 64,
        "backend": "macos"
    })
    .as_object()
    .cloned()
    .expect("object arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_run").with_arguments(arguments))
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["schema"], "soma.mcp.v1");
    assert_eq!(structured["operation"], "run");
    assert_eq!(structured["operation_id"], OPERATION_ID);
    assert_eq!(structured["result"]["stdout"]["encoding"], "base64");
    assert_eq!(structured["result"]["stdout"]["byte_length"], 4);
    assert_eq!(structured["result"]["stdout"]["data"], "/wBvaw==");
    assert_eq!(structured["result"]["stderr"]["data"], "/g==");
    assert_eq!(structured["receipt"]["schema_version"], 1);
    assert_eq!(structured["receipt"]["requested_shape"]["vcpu_count"], 2);
    assert_eq!(structured["receipt"]["requested_shape"]["memory_mib"], 2048);
    assert_eq!(
        structured["receipt"]["requested_shape"]["storage_mib"],
        8192
    );

    {
        let requests = requests.lock().expect("request ledger");
        let RuntimeRequest::Run(request) = requests.first().expect("run request") else {
            panic!("expected run request");
        };
        assert_eq!(request.image().as_str(), "node:22");
        assert_eq!(
            request.display_name().map(soma_mcp::DisplayName::as_str),
            Some("node-evaluator")
        );
        assert_eq!(request.operation_id().as_str(), OPERATION_ID);
        assert_eq!(request.instance_id().as_str(), INSTANCE_ID);
        assert_eq!(request.command().executable(), "/usr/local/bin/node");
        assert_eq!(
            request.command().arguments(),
            ["--eval", "process.stdout.write('ok')"]
        );
        assert_eq!(request.shape().vcpu_count(), 2);
        let network = request.shape().capabilities().network_policy();
        assert_eq!(network.egress(), EgressPolicy::Denied);
        assert_eq!(network.dns(), &DnsPolicy::Denied);
        assert_eq!(request.limits().max_output_bytes(), 64);
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
