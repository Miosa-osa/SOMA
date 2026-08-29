use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::DestroyInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = destroy_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_destroy",
        description = "Destroy a managed SOMA VM and require cleanup evidence. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Destroy",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn destroy(
        &self,
        Parameters(input): Parameters<DestroyInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::DestroyRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        match self.invoke_runtime(RuntimeRequest::Destroy(request)).await {
            Ok(RuntimeResponse::Destroy(result)) => {
                crate::result::machine_result("destroy", &operation_id, &instance_id, &result)
                    .map_err(|_| {
                        ErrorData::internal_error(
                            "SOMA runtime returned an invalid destroy result",
                            None,
                        )
                    })
            }
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "destroy",
                Some(&operation_id),
                Some(&instance_id),
                failure,
                None,
            )),
        }
    }
}
