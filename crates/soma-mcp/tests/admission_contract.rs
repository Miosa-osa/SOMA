use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rmcp::{ServiceExt as _, model::CallToolRequestParams};
use soma_mcp::{
    DoctorReport, DoctorStatus, RuntimeFailure, RuntimeRequest, RuntimeResponse, SomaMcpServer,
    ToolRuntime,
};
use tokio::sync::{Notify, Semaphore};

const MAX_IN_FLIGHT_TOOLS: usize = 32;

#[derive(Clone)]
struct BlockingRuntime {
    entered: Arc<AtomicUsize>,
    changed: Arc<Notify>,
    release: Arc<Semaphore>,
}

impl Default for BlockingRuntime {
    fn default() -> Self {
        Self {
            entered: Arc::new(AtomicUsize::new(0)),
            changed: Arc::new(Notify::new()),
            release: Arc::new(Semaphore::new(0)),
        }
    }
}

impl ToolRuntime for BlockingRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        let RuntimeRequest::Doctor { backend } = request else {
            panic!("admission test only invokes doctor");
        };
        self.entered.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
        self.release
            .acquire()
            .await
            .expect("test release semaphore")
            .forget();
        Ok(RuntimeResponse::Doctor(DoctorReport {
            backend,
            status: DoctorStatus::ProbePassed,
            supported_target: true,
            runtime_ready: true,
            production_ready: false,
        }))
    }
}

#[tokio::test]
async fn excess_concurrency_fails_without_an_unbounded_wait_queue() {
    let runtime = BlockingRuntime::default();
    let entered = Arc::clone(&runtime.entered);
    let changed = Arc::clone(&runtime.changed);
    let release = Arc::clone(&runtime.release);
    let (server_transport, client_transport) = tokio::io::duplex(256 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(runtime)
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = Arc::new(().serve(client_transport).await.expect("client connects"));

    let mut calls = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_TOOLS {
        let client = Arc::clone(&client);
        calls.push(tokio::spawn(async move {
            client
                .call_tool(CallToolRequestParams::new("soma_doctor"))
                .await
        }));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while entered.load(Ordering::SeqCst) != MAX_IN_FLIGHT_TOOLS {
            changed.notified().await;
        }
    })
    .await
    .expect("all bounded slots admitted");

    let excess = tokio::time::timeout(
        Duration::from_millis(500),
        client.call_tool(CallToolRequestParams::new("soma_doctor")),
    )
    .await
    .expect("excess call rejected without queueing");
    let excess = excess.expect("structured capacity response");
    assert_eq!(excess.is_error, Some(true));
    assert_eq!(
        excess.structured_content.expect("structured failure")["error"]["code"],
        "unavailable"
    );
    assert_eq!(entered.load(Ordering::SeqCst), MAX_IN_FLIGHT_TOOLS);

    release.add_permits(MAX_IN_FLIGHT_TOOLS);
    for call in calls {
        assert!(call.await.expect("call task").is_ok());
    }
    assert_eq!(entered.load(Ordering::SeqCst), MAX_IN_FLIGHT_TOOLS);

    Arc::try_unwrap(client)
        .expect("all call handles dropped")
        .cancel()
        .await
        .expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}
