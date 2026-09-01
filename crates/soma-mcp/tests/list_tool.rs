use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma::{
    BackendKind, InstanceId as SomaInstanceId, MachineName, SandboxEntry, SandboxLiveness,
    SandboxPhase,
};
use soma_mcp::{
    ListResult, RuntimeFailure, RuntimeRequest, RuntimeResponse, SomaMcpServer, ToolRuntime,
};

const INSTANCE_ID: &str = "22222222222222222222222222222222";

#[derive(Clone, Copy, Default)]
struct FakeRuntime;

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the test runtime has no asynchronous work"
)]
impl ToolRuntime for FakeRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        assert!(matches!(request, RuntimeRequest::List { .. }));
        Ok(RuntimeResponse::List(ListResult::new(vec![
            SandboxEntry::new(
                SomaInstanceId::new(INSTANCE_ID.to_owned()).expect("instance"),
                SandboxPhase::Active,
                BackendKind::LinuxKvm,
                Some(MachineName::parse("agent-body").expect("name")),
                SandboxLiveness::Live,
            ),
        ])))
    }
}

#[tokio::test]
async fn list_separates_durable_phase_from_observed_liveness() {
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(FakeRuntime)
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = ().serve(client_transport).await.expect("client connects");
    let arguments = serde_json::json!({"backend": "kvm"})
        .as_object()
        .cloned()
        .expect("arguments");

    let result = client
        .call_tool(CallToolRequestParams::new("soma_list").with_arguments(arguments))
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let body = result.structured_content.expect("structured result");
    assert_eq!(body["result"]["count"], 1);
    assert_eq!(body["result"]["sandboxes"][0]["instance_id"], INSTANCE_ID);
    assert_eq!(body["result"]["sandboxes"][0]["phase"], "active");
    assert_eq!(body["result"]["sandboxes"][0]["liveness"], "live");

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
