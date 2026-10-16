use std::borrow::Cow;

use crate::ToolRuntime;
use rmcp::{
    ServerHandler,
    model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    tool_handler,
};

use super::SomaMcpServer;

const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = ProtocolVersion::KNOWN_VERSIONS;

#[allow(
    clippy::unused_async_trait_impl,
    reason = "rmcp generates the required asynchronous handler implementation"
)]
#[tool_handler(router = self.tool_router)]
impl<R: ToolRuntime> ServerHandler for SomaMcpServer<R> {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("soma-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Use direct bounded SOMA tools. Commands are argv arrays and never implicit shells.",
            )
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }
}
