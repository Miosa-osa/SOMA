//! What a `/filesystem` call the runner serves itself answers with: the loopback route's own
//! envelope for a result, and its failure body for a refusal.

use soma::{FileAnswer, InstanceId};

use crate::{
    capability::MissingCapability,
    envelope::{ApiError, Envelope, failure_body, render},
    failure::managed_error,
    filesystem::{FilesystemBody, FilesystemReport},
    route::FilesystemOperation,
    runner::files::{Stop, write_body::BodyRefused},
};

use super::RunnerResponse;

/// The operation every `/filesystem` answer names.
pub(super) const OPERATION: &str = "sandbox.filesystem";

/// Why a large write or read did not complete.
pub(super) enum Failed {
    Body(BodyRefused),
    /// The content began before the path, so the file had nowhere to be staged.
    PathAfterContent,
    NoContent,
    Interrupted,
    Api(ApiError),
    Stopped(Stop),
    Busy,
}

impl Failed {
    /// The answer the loopback route gives for the same failure.
    pub(super) fn answer(self, instance_id: InstanceId, operation: &'static str) -> RunnerResponse {
        match self {
            Self::Body(BodyRefused::Content) => {
                refused(ApiError::invalid("the content field is not valid base64"))
            }
            Self::Body(BodyRefused::Malformed) | Self::Interrupted => refused(body_error()),
            Self::PathAfterContent => refused(ApiError::new(
                413,
                "content_too_large",
                "a write larger than one transfer must name its path before its content",
                false,
            )),
            Self::NoContent => {
                refused(ApiError::invalid("a write must carry the content to write"))
            }
            Self::Api(error) => refused(error),
            Self::Stopped(Stop::Refused(refusal)) => {
                answer(instance_id, operation, &FileAnswer::Refused(refusal))
            }
            Self::Stopped(Stop::Failed(soma::ManagedFailure::Backend(
                soma::BackendFailureKind::Unsupported,
            ))) => refused(MissingCapability::GuestFilesystemTransfer.error()),
            Self::Stopped(Stop::Failed(failure)) => refused(managed_error(&failure)),
            Self::Stopped(Stop::Unexpected) => refused(ApiError::internal(
                "the guest answered a file operation out of kind",
            )),
            Self::Busy => RunnerResponse::runtime_busy(),
        }
    }
}

/// A body that is not the document the route reads, in the loopback's words.
pub(super) fn body_error() -> ApiError {
    ApiError::invalid("the request body is not a valid document for this route")
}

/// The fields other than `content`, checked as the loopback route checks them.
pub(super) fn validate(fields: &[u8], instance_id: &InstanceId) -> Result<(), ApiError> {
    let body: FilesystemBody = serde_json::from_slice(fields).map_err(|_| body_error())?;
    body.into_facade(instance_id.clone(), FilesystemOperation::Write)
        .map(|_| ())
}

/// One `/filesystem` answer in the loopback's envelope.
pub(super) fn answer(
    instance_id: InstanceId,
    operation: &'static str,
    file: &FileAnswer,
) -> RunnerResponse {
    let report = FilesystemReport::new(instance_id, operation, file);
    RunnerResponse::new(200, render(&Envelope::success(OPERATION, &report, None)))
}

/// One `/filesystem` refusal in the loopback's failure body.
pub(super) fn refused(error: ApiError) -> RunnerResponse {
    RunnerResponse::new(error.status(), failure_body(OPERATION, error))
}
