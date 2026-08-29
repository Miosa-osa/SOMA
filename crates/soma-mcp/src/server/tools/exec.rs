use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::ExecInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = exec_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_exec",
        description = "Execute one bounded direct argv command in a managed SOMA VM. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Exec",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn exec(
        &self,
        Parameters(input): Parameters<ExecInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::ExecRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        let max_output_bytes = request.limits().max_output_bytes();
        match self.invoke_runtime(RuntimeRequest::Exec(request)).await {
            Ok(RuntimeResponse::Exec(result)) => crate::result::command_result(
                "exec",
                &operation_id,
                &instance_id,
                &result,
                max_output_bytes,
            )
            .map_err(|_| {
                ErrorData::internal_error("SOMA runtime returned an invalid bounded result", None)
            }),
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "exec",
                Some(&operation_id),
                Some(&instance_id),
                &failure,
                Some(max_output_bytes),
            )),
        }
    }
}
