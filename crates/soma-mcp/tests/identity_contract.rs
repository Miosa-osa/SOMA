use std::sync::{Arc, Mutex};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{
    CommandResult, CommandStatus, ExecutionReceipt, RuntimeFailure, RuntimeRequest,
    RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const OPERATION_ID: &str = "11111111111111111111111111111111";
const INSTANCE_ID: &str = "22222222222222222222222222222222";

#[derive(Clone, Default)]
struct EchoRuntime {
    requests: Arc<Mutex<Vec<RuntimeRequest>>>,
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the synchronous test double implements the asynchronous runtime seam"
)]
impl ToolRuntime for EchoRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        let RuntimeRequest::Run(run) = &request else {
            panic!("identity test only invokes run");
        };
        let result = CommandResult::new(
            run.instance_id().clone(),
            CommandStatus::Exited { code: 0 },
            Vec::new(),
            Vec::new(),
            ExecutionReceipt::new(serde_json::json!({ "schema_version": 1 })).expect("receipt"),
        );
        self.requests.lock().expect("request ledger").push(request);
        Ok(RuntimeResponse::Run(result))
    }
}

fn run_arguments() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "image": "ubuntu:24.04",
        "executable": "/bin/true"
    })
    .as_object()
    .cloned()
    .expect("object arguments")
}

async fn start(
    runtime: EchoRuntime,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tokio::task::JoinHandle<Result<rmcp::service::QuitReason, tokio::task::JoinError>>,
) {
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
    (client, server_task)
}

#[tokio::test]
async fn omitted_ids_are_secure_canonical_ids_returned_to_the_agent() {
    let runtime = EchoRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (client, server_task) = start(runtime).await;

    let result = client
        .call_tool(CallToolRequestParams::new("soma_run").with_arguments(run_arguments()))
        .await
        .expect("tool response");

    let structured = result.structured_content.expect("structured result");
    let operation_id = structured["operation_id"].as_str().expect("operation ID");
    let instance_id = structured["result"]["instance_id"]
        .as_str()
        .expect("instance ID");
    for id in [operation_id, instance_id] {
        assert_eq!(id.len(), 32);
        assert_ne!(id, "00000000000000000000000000000000");
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(id.as_bytes()[12], b'4');
        assert!(matches!(id.as_bytes()[16], b'8' | b'9' | b'a' | b'b'));
    }
    {
        let requests = requests.lock().expect("request ledger");
        let RuntimeRequest::Run(request) = requests.first().expect("run request") else {
            panic!("expected run request");
        };
        assert_eq!(request.operation_id().as_str(), operation_id);
        assert_eq!(request.instance_id().as_str(), instance_id);
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}

#[tokio::test]
async fn explicit_ids_preserve_retry_intent_without_static_idempotency_claims() {
    let runtime = EchoRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (client, server_task) = start(runtime).await;
    let mut arguments = run_arguments();
    arguments.insert("operation_id".into(), OPERATION_ID.into());
    arguments.insert("instance_id".into(), INSTANCE_ID.into());

    for _ in 0..2 {
        let result = client
            .call_tool(CallToolRequestParams::new("soma_run").with_arguments(arguments.clone()))
            .await
            .expect("tool response");
        let structured = result.structured_content.expect("structured result");
        assert_eq!(structured["operation_id"], OPERATION_ID);
        assert_eq!(structured["result"]["instance_id"], INSTANCE_ID);
    }
    {
        let requests = requests.lock().expect("request ledger");
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            let RuntimeRequest::Run(request) = request else {
                panic!("expected run request");
            };
            assert_eq!(request.operation_id().as_str(), OPERATION_ID);
            assert_eq!(request.instance_id().as_str(), INSTANCE_ID);
        }
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}

#[tokio::test]
async fn malformed_and_all_zero_ids_fail_before_runtime_admission() {
    let runtime = EchoRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (client, server_task) = start(runtime).await;

    for (field, value) in [
        ("operation_id", "00000000000000000000000000000000"),
        ("instance_id", "00000000000000000000000000000000"),
        ("instance_id", "2222222222222222222222222222222A"),
        ("instance_id", "2222"),
    ] {
        let mut arguments = run_arguments();
        arguments.insert(field.into(), value.into());
        assert!(
            client
                .call_tool(CallToolRequestParams::new("soma_run").with_arguments(arguments))
                .await
                .is_err()
        );
    }
    assert!(requests.lock().expect("request ledger").is_empty());

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}

#[tokio::test]
async fn portable_contract_rejects_pathological_requests_before_runtime() {
    let runtime = EchoRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (client, server_task) = start(runtime).await;
    let oversized_argument = "x".repeat(128 * 1024 + 1);
    let too_many_arguments = vec![""; 4097];
    let aggregate_arguments = vec!["x".repeat(128 * 1024); 9];
    let invalid_overrides = vec![
        serde_json::json!({ "vcpu_count": 0 }),
        serde_json::json!({ "memory_mib": 0 }),
        serde_json::json!({ "timeout_ms": 86_400_001 }),
        serde_json::json!({ "max_output_bytes": 16_777_217 }),
        serde_json::json!({ "network": {"egress": "sometimes"} }),
        serde_json::json!({ "display_name": "Uppercase" }),
        serde_json::json!({ "image": "https://registry.example/image" }),
        serde_json::json!({ "executable": "bin/sh" }),
        serde_json::json!({ "arguments": [oversized_argument] }),
        serde_json::json!({ "arguments": too_many_arguments }),
        serde_json::json!({ "arguments": aggregate_arguments }),
    ];

    for (index, overrides) in invalid_overrides.into_iter().enumerate() {
        let mut arguments = run_arguments();
        arguments.extend(overrides.as_object().expect("override object").clone());
        let response = client
            .call_tool(CallToolRequestParams::new("soma_run").with_arguments(arguments))
            .await;
        let rejected = response
            .as_ref()
            .is_ok_and(|result| result.is_error == Some(true))
            || response.is_err();
        assert!(rejected, "invalid override {index} was accepted");
    }
    assert!(requests.lock().expect("request ledger").is_empty());

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
