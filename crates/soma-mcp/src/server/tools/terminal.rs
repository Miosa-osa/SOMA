use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, terminal::TerminalInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = terminal_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_terminal",
        description = "Open, write, read, resize, or close the bounded terminal session inside one managed SOMA VM. Input and output are base64 so terminal control bytes remain exact. The session remains inside the VM between calls. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Terminal",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn terminal(
        &self,
        Parameters(input): Parameters<TerminalInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::TerminalRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        match self.invoke_runtime(RuntimeRequest::Terminal(request)).await {
            Ok(RuntimeResponse::Terminal(result)) => {
                crate::result::terminal_result(&operation_id, &instance_id, &result).map_err(|_| {
                    ErrorData::internal_error(
                        "SOMA runtime returned an invalid terminal result",
                        None,
                    )
                })
            }
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "terminal",
                Some(&operation_id),
                Some(&instance_id),
                &failure,
                None,
            )),
        }
    }
}
