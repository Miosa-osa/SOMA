use crate::{RuntimeRequest, RuntimeResponse, ToolRuntime, input::DoctorInput};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router,
};

use super::super::{SomaMcpServer, failure::runtime_failure_result};

#[tool_router(router = doctor_router, vis = "pub(super)")]
impl<R: ToolRuntime> SomaMcpServer<R> {
    #[tool(
        name = "soma_doctor",
        description = "Probe SOMA without claiming production readiness. macOS is development-only and unsupported backends fail closed.",
        annotations(
            title = "SOMA Doctor",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn doctor(
        &self,
        Parameters(input): Parameters<DoctorInput>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .invoke_runtime(RuntimeRequest::Doctor {
                backend: input.backend.into(),
            })
            .await
        {
            Ok(RuntimeResponse::Doctor(report)) => crate::result::doctor_result(&report)
                .map_err(|_| ErrorData::internal_error("failed to encode SOMA response", None)),
            Ok(_) => Err(super::super::failure::wrong_result_type()),
            Err(failure) => Ok(runtime_failure_result("doctor", None, None, failure, None)),
        }
    }
}
