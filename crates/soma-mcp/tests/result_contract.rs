use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{
    CommandResult, CommandStatus, ExecutionReceipt, InstanceId, RuntimeFailure, RuntimeRequest,
    RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const REQUEST_INSTANCE_ID: &str = "22222222222222222222222222222222";
const OTHER_INSTANCE_ID: &str = "33333333333333333333333333333333";

#[derive(Clone, Copy)]
enum InvalidResultRuntime {
    WrongInstance,
    ExcessOutput,
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the synchronous test double implements the asynchronous runtime seam"
)]
impl ToolRuntime for InvalidResultRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        let RuntimeRequest::Run(run) = request else {
            panic!("result contract test only invokes run");
        };
        let (instance_id, stdout) = match self {
            Self::WrongInstance => (
                InstanceId::new(OTHER_INSTANCE_ID).expect("instance ID"),
                Vec::new(),
            ),
            Self::ExcessOutput => (run.instance_id().clone(), vec![b'x'; 65]),
        };
        Ok(RuntimeResponse::Run(CommandResult::new(
            instance_id,
            CommandStatus::Exited { code: 0 },
            stdout,
            Vec::new(),
            ExecutionReceipt::new(serde_json::json!({ "schema_version": 1 })).expect("receipt"),
        )))
    }
}

fn run_arguments() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "instance_id": REQUEST_INSTANCE_ID,
        "image": "ubuntu:24.04",
        "executable": "/bin/true",
        "max_output_bytes": 64
    })
    .as_object()
    .cloned()
    .expect("object arguments")
}

#[tokio::test]
async fn runtime_cannot_substitute_identity_or_exceed_agent_output_allowance() {
    for runtime in [
        InvalidResultRuntime::WrongInstance,
        InvalidResultRuntime::ExcessOutput,
    ] {
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

        assert!(
            client
                .call_tool(CallToolRequestParams::new("soma_run").with_arguments(run_arguments()),)
                .await
                .is_err()
        );

        client.cancel().await.expect("client closes");
        server_task
            .await
            .expect("server task")
            .expect("server closes");
    }
}

#[test]
fn receipt_must_be_a_bounded_json_object() {
    assert!(ExecutionReceipt::new(serde_json::json!(["not", "an", "object"])).is_err());
    assert!(
        ExecutionReceipt::new(serde_json::json!({
            "oversized": "x".repeat(256 * 1024)
        }))
        .is_err()
    );
}
