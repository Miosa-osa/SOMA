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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscardBody {
    checkpoint_id: OperationId,
    state_digest: soma::GenerationId,
}

pub(super) fn discard<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &InstanceId,
    body: &[u8],
) -> Result<Response, ApiError> {
    let body: DiscardBody = super::parse(body)?;
    facade
        .discard_checkpoint(instance, &body.checkpoint_id, &body.state_digest)
        .map_err(|failure| match failure {
            CheckpointFailure::Unsupported => ApiError::new(
                409,
                "checkpoint_discard_unsupported",
                "checkpoint storage is unavailable",
                false,
            ),
            CheckpointFailure::CaptureRefused => ApiError::new(
                409,
                "checkpoint_identity_mismatch",
                "checkpoint receipt identity did not match",
                false,
            ),
            _ => ApiError::new(
                503,
                "checkpoint_discard_unconfirmed",
                "the host did not confirm checkpoint removal",
                true,
            ),
        })?;
    Ok(super::success(
        200,
        "sandbox.checkpoint.discard",
        &serde_json::json!({
            "instance_id": instance,
            "checkpoint_id": body.checkpoint_id,
            "state": "absent",
        }),
        None,
    ))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreBody {
    operation_id: OperationId,
    checkpoint_id: OperationId,
    image: String,
    shape: soma::MachineShape,
    name: Option<soma::MachineName>,
}

pub(super) fn restore<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: InstanceId,
    body: &[u8],
) -> Result<Response, ApiError> {
    try_restore(facade, instance, body, None)
}

pub(crate) fn restore_as<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: InstanceId,
    body: &[u8],
    owner: soma::MachineName,
) -> Response {
    try_restore(facade, instance, body, Some(owner))
        .unwrap_or_else(|error| super::refuse("sandbox.restore", error))
}

fn try_restore<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: InstanceId,
    body: &[u8],
    owner: Option<soma::MachineName>,
) -> Result<Response, ApiError> {
    if !facade.hosts_addressable_sandboxes() {
        return Err(super::MissingCapability::DurableMachineHosting.error());
    }
    let body: RestoreBody = super::parse(body)?;
    let image = soma::OciImage::parse(body.image)
        .map_err(|_| ApiError::invalid("the image reference is not a valid OCI reference"))?;
    let mut request =
        soma::LaunchMachineRequest::new(body.operation_id, instance, image, body.shape);
    if let Some(name) = owner.or(body.name) {
        request = request.with_name(name);
    }
    let result = facade
        .restore(request, &body.checkpoint_id)
        .map_err(|failure| super::managed_error(&failure))?;
    Ok(super::lifecycle(
        "sandbox.restore",
        "ready",
        &result.instance_id,
        &result.receipt,
    ))
}

/// The caller supplies the child identity from its committed placement record.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForkBody {
    operation_id: OperationId,
    checkpoint_id: OperationId,
    instance_id: InstanceId,
    image: String,
    shape: soma::MachineShape,
    name: soma::MachineName,
}

pub(super) fn fork<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    source: &InstanceId,
    body: &[u8],
) -> Result<Response, ApiError> {
    if !facade.hosts_addressable_sandboxes() {
        return Err(super::MissingCapability::DurableMachineHosting.error());
    }
    let body: ForkBody = super::parse(body)?;
    if &body.instance_id == source {
        return Err(ApiError::invalid(
            "fork requires a distinct destination instance",
        ));
    }
    let image = soma::OciImage::parse(body.image)
        .map_err(|_| ApiError::invalid("the image reference is not a valid OCI reference"))?;
    let request =
        soma::LaunchMachineRequest::new(body.operation_id, body.instance_id, image, body.shape)
            .with_name(body.name);
    let result = facade
        .fork(request, source, &body.checkpoint_id)
        .map_err(|failure| super::managed_error(&failure))?;
    Ok(super::lifecycle(
        "sandbox.fork",
        "ready",
        &result.instance_id,
        &result.receipt,
    ))
}

pub(crate) fn fork_as<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    request: soma::LaunchMachineRequest,
    source: &InstanceId,
    checkpoint: &OperationId,
    sandbox_id: &str,
) -> Response {
    match facade.fork(request, source, checkpoint) {
        Ok(result) => super::success(
            200,
            "sandbox.fork",
            &serde_json::json!({
                "sandbox_id": sandbox_id,
                "state": "ready",
                "receipt": result.receipt,
            }),
            None,
        ),
        Err(failure) => super::refuse("sandbox.fork", super::managed_error(&failure)),
    }
}
