use std::sync::{Arc, Mutex};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma::{PtyAnswer, PtyOperation};
use soma_mcp::{
    InstanceId, RuntimeFailure, RuntimeRequest, RuntimeResponse, SomaMcpServer, TerminalResult,
    ToolRuntime,
};

const OPERATION_ID: &str = "33333333333333333333333333333333";
const INSTANCE_ID: &str = "22222222222222222222222222222222";

#[derive(Clone, Default)]
struct FakeRuntime {
    requests: Arc<Mutex<Vec<RuntimeRequest>>>,
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the test runtime has no asynchronous work"
)]
impl ToolRuntime for FakeRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        self.requests.lock().expect("request ledger").push(request);
        Ok(RuntimeResponse::Terminal(TerminalResult::new(
            InstanceId::new(INSTANCE_ID).expect("instance"),
            "read",
            PtyAnswer::Output {
                bytes: vec![0, 255],
                end: false,
            },
        )))
    }
}

#[tokio::test]
async fn terminal_preserves_bounded_binary_calls_and_answers() {
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
        "operation": "read",
        "wait_ms": 25,
        "backend": "kvm"
    })
    .as_object()
    .cloned()
    .expect("arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_terminal").with_arguments(arguments))
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let body = result.structured_content.expect("structured result");
    assert_eq!(body["operation"], "terminal");
    assert_eq!(body["result"]["output"]["data"], "AP8=");
    assert_eq!(body["result"]["ended"], false);
    {
        let mut requests = requests.lock().expect("request ledger");
        let RuntimeRequest::Terminal(request) = requests.pop().expect("request") else {
            panic!("expected terminal request");
        };
        assert_eq!(request.operation_id().as_str(), OPERATION_ID);
        assert_eq!(
            request.into_facade().operation(),
            &PtyOperation::Read { wait_millis: 25 }
        );
    }

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
