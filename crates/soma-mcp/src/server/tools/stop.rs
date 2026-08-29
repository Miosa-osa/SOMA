use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::StopInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = stop_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_stop",
        description = "Stop a managed SOMA VM and return cleanup evidence. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Stop",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn stop(
        &self,
        Parameters(input): Parameters<StopInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::StopRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        match self.invoke_runtime(RuntimeRequest::Stop(request)).await {
            Ok(RuntimeResponse::Stop(result)) => crate::result::machine_result(
                "stop",
                &operation_id,
                &instance_id,
                &result,
            )
            .map_err(|_| {
                ErrorData::internal_error("SOMA runtime returned an invalid stop result", None)
            }),
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "stop",
                Some(&operation_id),
                Some(&instance_id),
                &failure,
                None,
            )),
        }
    }
}
