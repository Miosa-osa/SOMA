use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, file::FileInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[tool_router(router = file_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_file",
        description = "Read, write, list, stat, remove, or create a directory inside a managed SOMA VM. Paths are absolute and validated by the guest; content is base64 in both directions so binary files survive unchanged. macOS is development-only and holds no machine a later call can reach, so unsupported backends fail closed.",
        annotations(
            title = "SOMA File",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn file(
        &self,
        Parameters(input): Parameters<FileInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = crate::FileRequest::try_from(input)
            .map_err(|error| ErrorData::invalid_params(error.message(), None))?;
        let operation_id = request.operation_id().clone();
        let instance_id = request.instance_id().clone();
        match self.invoke_runtime(RuntimeRequest::File(request)).await {
            Ok(RuntimeResponse::File(result)) => {
                crate::result::file_result(&operation_id, &instance_id, &result).map_err(|_| {
                    ErrorData::internal_error(
                        "SOMA runtime returned an invalid bounded result",
                        None,
                    )
                })
            }
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result(
                "file",
                Some(&operation_id),
                Some(&instance_id),
                &failure,
                None,
            )),
        }
    }
}
