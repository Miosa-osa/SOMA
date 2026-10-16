use std::sync::{Arc, Mutex};

use rmcp::{
    ClientHandler, ServiceExt as _,
    model::{CallToolRequestParams, ProtocolVersion},
};
use soma_mcp::{
    BackendTarget, DoctorReport, DoctorStatus, RuntimeFailure, RuntimeRequest, RuntimeResponse,
    SomaMcpServer, ToolRuntime,
};

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
        self.requests
            .lock()
            .expect("request ledger")
            .push(request.clone());
        Ok(RuntimeResponse::Doctor(DoctorReport {
            backend: BackendTarget::Macos,
            status: DoctorStatus::ProbePassed,
            supported_target: true,
            runtime_ready: true,
            production_ready: false,
        }))
    }
}

#[tokio::test]
async fn agent_can_probe_a_backend_through_mcp() {
    let runtime = FakeRuntime::default();
    let requests = Arc::clone(&runtime.requests);
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
    let server = SomaMcpServer::new(runtime);
    let server_task = tokio::spawn(async move {
        server
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = ().serve(client_transport).await.expect("client connects");

    let result = client
        .call_tool(
            CallToolRequestParams::new("soma_doctor").with_arguments(
                serde_json::json!({ "backend": "macos" })
                    .as_object()
                    .cloned()
                    .expect("object arguments"),
            ),
        )
        .await
        .expect("tool response");

    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured result");
    assert_eq!(structured["schema"], "soma.mcp.v1");
    assert_eq!(structured["operation"], "doctor");
    assert_eq!(structured["operation_id"], serde_json::Value::Null);
    assert_eq!(structured["result"]["runtime_ready"], true);
    assert_eq!(structured["receipt"], serde_json::Value::Null);
    assert_eq!(
        requests.lock().expect("request ledger").as_slice(),
        [RuntimeRequest::Doctor {
            backend: BackendTarget::Macos
        }]
    );
    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}

#[derive(Clone)]
struct VersionClient(ProtocolVersion);

impl ClientHandler for VersionClient {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::new(
            rmcp::model::ClientCapabilities::default(),
            rmcp::model::Implementation::new("soma-mcp-test", "1"),
        )
        .with_protocol_version(self.0.clone())
    }
}

#[tokio::test]
async fn legacy_and_current_clients_negotiate_their_requested_versions() {
    for expected in [ProtocolVersion::V_2024_11_05, ProtocolVersion::V_2026_07_28] {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            SomaMcpServer::new(FakeRuntime::default())
                .serve(server_transport)
                .await
                .expect("server starts")
                .waiting()
                .await
        });
        let client = VersionClient(expected.clone())
            .serve(client_transport)
            .await
            .expect("client connects");

        assert_eq!(
            client.peer_info().expect("server info").protocol_version,
            expected
        );
        client.cancel().await.expect("client closes");
        server_task
            .await
            .expect("server task")
            .expect("server closes");
    }
}
