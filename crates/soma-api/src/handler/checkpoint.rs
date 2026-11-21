use soma::{InstanceId, OperationId};
use soma_local::CheckpointFailure;

use crate::{envelope::ApiError, facade::SandboxFacade, http::response::Response};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckpointBody {
    operation_id: OperationId,
}

pub(super) fn serve<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &InstanceId,
    body: &[u8],
) -> Result<Response, ApiError> {
    let body: CheckpointBody = super::parse(body)?;
    let receipt =
        facade
            .checkpoint(instance, &body.operation_id)
            .map_err(|failure| match failure {
                CheckpointFailure::NotFound => ApiError::not_found(),
                CheckpointFailure::Unsupported => ApiError::new(
                    409,
                    "checkpoint_unsupported",
                    "checkpoint capture is unavailable for this sandbox",
                    false,
                ),
                CheckpointFailure::CaptureRefused => ApiError::new(
                    409,
                    "checkpoint_refused",
                    "the sandbox did not meet checkpoint capture requirements",
                    false,
                ),
                CheckpointFailure::Storage => ApiError::new(
                    503,
                    "checkpoint_storage_unavailable",
                    "the host did not confirm checkpoint publication",
                    true,
                ),
                CheckpointFailure::Unconfirmed => ApiError::new(
                    503,
                    "checkpoint_unconfirmed",
                    "the host did not confirm the sandbox state",
                    true,
                ),
            })?;
    Ok(super::success(200, "sandbox.checkpoint", &receipt, None))
}
