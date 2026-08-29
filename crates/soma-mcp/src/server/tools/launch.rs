use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::LaunchInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = launch_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_launch",
        description = "Launch a managed SOMA VM from an OCI image. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Launch",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn launch(
        &self,
        Parameters(input): Parameters<LaunchInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::LaunchRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        match self.invoke_runtime(RuntimeRequest::Launch(request)).await {
            Ok(RuntimeResponse::Launch(result)) => {
                crate::result::machine_result("launch", &operation_id, &instance_id, &result)
                    .map_err(|_| {
                        ErrorData::internal_error(
                            "SOMA runtime returned an invalid managed result",
                            None,
                        )
                    })
            }
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "launch",
                Some(&operation_id),
                Some(&instance_id),
                failure,
                None,
            )),
        }
    }
}
