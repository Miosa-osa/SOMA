use crate::{
    envelope::ApiError, facade::SandboxFacade, http::response::Response, report::SandboxReport,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResidentBody {}

#[derive(Clone, Copy)]
pub(super) enum Transition {
    Pause,
    Resume,
}

pub(super) fn serve<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance_id: soma::InstanceId,
    body: &[u8],
    transition: Transition,
) -> Result<Response, ApiError> {
    if !body.is_empty() {
        let _: ResidentBody = super::parse(body)?;
    }
    let (result, operation, state, code, message) = match transition {
        Transition::Pause => (
            facade.pause(&instance_id),
            "sandbox.pause",
            "paused",
            "pause_unavailable",
            "the sandbox host did not confirm pause",
        ),
        Transition::Resume => (
            facade.resume(&instance_id),
            "sandbox.resume",
            "running",
            "resume_unavailable",
            "the sandbox host did not confirm resume",
        ),
    };
    result.map_err(|failure| match failure {
        soma_local::PauseFailure::NotFound => ApiError::not_found(),
        soma_local::PauseFailure::Unavailable => ApiError::new(503, code, message, true),
    })?;
    Ok(super::success(
        200,
        operation,
        &SandboxReport {
            instance_id,
            state,
            volume: None,
            fsck: None,
        },
        None,
    ))
}
