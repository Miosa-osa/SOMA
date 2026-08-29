use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::InspectInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = inspect_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_inspect",
        description = "Inspect bounded managed SOMA state and its latest receipt. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Inspect",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn inspect(
        &self,
        Parameters(input): Parameters<InspectInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::InspectRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        match self.invoke_runtime(RuntimeRequest::Inspect(request)).await {
            Ok(RuntimeResponse::Inspect(result)) => {
                crate::result::inspect_result(&operation_id, &instance_id, &result).map_err(|_| {
                    ErrorData::internal_error(
                        "SOMA runtime returned an invalid inspection result",
                        None,
                    )
                })
            }
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "inspect",
                Some(&operation_id),
                Some(&instance_id),
                failure,
                None,
            )),
        }
    }
}
