mod admission;
mod failure;
mod protocol;
mod tools;

use std::sync::Arc;

use crate::ToolRuntime;
use rmcp::handler::server::router::tool::ToolRouter;

/// Protocol work admitted concurrently before the runtime's shape-aware capacity gate decides
/// whether a sandbox itself fits.
///
/// This must cover the public 100-way burst without becoming the machine-capacity policy.
/// The runtime still owns CPU, memory, storage, and process admission for every Launch.
const MAX_IN_FLIGHT_TOOLS: usize = 128;

/// Bounded Model Context Protocol access to one SOMA runtime adapter.
pub struct SomaMcpServer<R> {
    runtime: Arc<R>,
    admission: Arc<tokio::sync::Semaphore>,
    tool_router: ToolRouter<Self>,
}

impl<R> Clone for SomaMcpServer<R> {
    fn clone(&self) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
            admission: Arc::clone(&self.admission),
            tool_router: self.tool_router.clone(),
        }
    }
}

impl<R: ToolRuntime> SomaMcpServer<R> {
    #[must_use]
    pub fn new(runtime: R) -> Self {
        Self {
            runtime: Arc::new(runtime),
            admission: Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT_TOOLS)),
            tool_router: tools::router(),
        }
    }
}
