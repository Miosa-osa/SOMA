use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::RunInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = run_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_run",
        description = "Run one bounded direct argv command in a fresh SOMA VM and clean it up. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Run",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn run(
        &self,
        Parameters(input): Parameters<RunInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::RunRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        let max_output_bytes = request.limits().max_output_bytes();
        match self.invoke_runtime(RuntimeRequest::Run(request)).await {
            Ok(RuntimeResponse::Run(result)) => crate::result::command_result(
                "run",
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
                "run",
                Some(&operation_id),
                Some(&instance_id),
                &failure,
                Some(max_output_bytes),
            )),
        }
    }
}
