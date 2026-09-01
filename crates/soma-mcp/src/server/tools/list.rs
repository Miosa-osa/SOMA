use crate::{BackendTarget, RuntimeRequest, RuntimeResponse, ToolRuntime, input::BackendInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use super::super::{
    SomaMcpServer,
    failure::{runtime_failure_result, wrong_result_type},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    #[serde(default)]
    #[schemars(default)]
    backend: BackendInput,
}

#[tool_router(router = list_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_list",
        description = "List every managed SOMA sandbox that has not been destroyed. Durable lifecycle phase and currently observed backend liveness are reported separately so stale state is never presented as a live VM. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA List",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list(
        &self,
        Parameters(input): Parameters<ListInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let backend = BackendTarget::from(input.backend);
        match self.invoke_runtime(RuntimeRequest::List { backend }).await {
            Ok(RuntimeResponse::List(result)) => {
                crate::result::list_result(&result).map_err(|_| {
                    ErrorData::internal_error("SOMA runtime returned an invalid listing", None)
                })
            }
            Ok(_) => Err(wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result("list", None, None, &failure, None)),
        }
    }
}
