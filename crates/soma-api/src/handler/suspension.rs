//! The suspension routes (ADR 0034): suspend a running sandbox into a sealed Suspension, resume
//! one as a new sandbox, and discard one.

use serde::{Deserialize, Serialize};
use soma::{InstanceId, OciImage, OperationId};
use soma_local::{ResumeRequest, SuspendRequest, SuspensionFailure};

use super::support::{optional_body, success};
use crate::capability::MissingCapability;
use crate::envelope::ApiError;
use crate::facade::SandboxFacade;
use crate::failure::managed_error;
use crate::http::response::Response;
use crate::wire::operation_id;

/// How long a Suspension is kept when the caller does not say: seven days.
const DEFAULT_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuspendBody {
    #[serde(default)]
    operation_id: Option<OperationId>,
    /// The image the sandbox was created from, kept so a resume need not name it.
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    retention_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResumeBody {
    #[serde(default)]
    operation_id: Option<OperationId>,
    /// The new sandbox's identity; one is generated when absent.
    #[serde(default)]
    instance_id: Option<InstanceId>,
    #[serde(default)]
    image: Option<String>,
}

#[derive(Serialize)]
struct SuspendedReport {
    instance_id: InstanceId,
    state: &'static str,
    suspension_id: String,
    expires_unix: u64,
    sealed_bytes: u64,
}

#[derive(Serialize)]
struct ResumedReport {
    instance_id: InstanceId,
    state: &'static str,
    resumed_from: String,
}

#[derive(Serialize)]
struct DiscardedReport {
    suspension_id: String,
    state: &'static str,
}

fn image(reference: Option<String>) -> Result<Option<OciImage>, ApiError> {
    reference
        .map(OciImage::parse)
        .transpose()
        .map_err(|_| ApiError::invalid("the image reference is not a valid OCI reference"))
}

pub(super) fn suspend<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance_id: &InstanceId,
    tenant: &str,
    body: &[u8],
) -> Result<Response, ApiError> {
    let body = optional_body::<SuspendBody>(body)?;
    let request = SuspendRequest {
        operation_id: operation_id(body.operation_id)?,
        instance_id: instance_id.clone(),
        tenant: tenant.to_owned(),
        image: image(body.image)?,
        retention_secs: body.retention_secs.unwrap_or(DEFAULT_RETENTION_SECS),
    };
    let suspended = facade
        .suspend(&request)
        .map_err(|failure| refusal(&failure))?;
    Ok(success(
        200,
        "sandbox.suspend",
        &SuspendedReport {
            instance_id: instance_id.clone(),
            state: "suspended",
            suspension_id: suspended.suspension_id,
            expires_unix: suspended.expires_unix,
            sealed_bytes: suspended.sealed_bytes,
        },
        Some(&suspended.receipt),
    ))
}

pub(super) fn resume<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    suspension_id: &str,
    body: &[u8],
) -> Result<Response, ApiError> {
    let body = optional_body::<ResumeBody>(body)?;
    if !facade.hosts_addressable_sandboxes() {
        return Err(MissingCapability::DurableMachineHosting.error());
    }
    let instance_id = match body.instance_id {
        Some(instance_id) => instance_id,
        None => InstanceId::new(uuid::Uuid::new_v4().simple().to_string()).map_err(|_| {
            ApiError::internal("a generated instance identity was rejected by the facade")
        })?,
    };
    let request = ResumeRequest {
        operation_id: operation_id(body.operation_id)?,
        instance_id,
        suspension_id: suspension_id.to_owned(),
        image: image(body.image)?,
    };
    let outcome = facade
        .resume(&request)
        .map_err(|failure| refusal(&failure))?;
    Ok(success(
        201,
        "suspension.resume",
        &ResumedReport {
            instance_id: outcome.instance_id,
            state: "ready",
            resumed_from: suspension_id.to_owned(),
        },
        Some(&outcome.receipt),
    ))
}

pub(super) fn destroy<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    suspension_id: &str,
) -> Result<Response, ApiError> {
    facade
        .discard_suspension(suspension_id)
        .map_err(|failure| refusal(&failure))?;
    Ok(success(
        200,
        "suspension.destroy",
        &DiscardedReport {
            suspension_id: suspension_id.to_owned(),
            state: "destroyed",
        },
        None,
    ))
}

fn refusal(failure: &SuspensionFailure) -> ApiError {
    match failure {
        SuspensionFailure::Managed(failure) => managed_error(failure),
        SuspensionFailure::Unknown => ApiError::new(
            404,
            "suspension_not_found",
            "no suspension of that identity is held on this host",
            false,
        ),
        SuspensionFailure::Expired => ApiError::new(
            410,
            "suspension_expired",
            "the suspension expired and has been destroyed",
            false,
        ),
        SuspensionFailure::OtherHost => ApiError::new(
            409,
            "suspension_other_host",
            "the suspension was taken on a host with another processor and cannot resume here",
            false,
        ),
        SuspensionFailure::NoImage => ApiError::invalid(
            "the suspension kept no image; name the image the sandbox was created from",
        ),
        SuspensionFailure::Lost(receipt) => {
            eprintln!(
                "soma-api: suspension of {} failed after the read began; the sandbox has ended",
                receipt.instance_id().as_str()
            );
            ApiError::new(
                500,
                "suspension_failed",
                "the sandbox could not be read into a suspension and has ended",
                false,
            )
        }
        SuspensionFailure::Store(detail) => {
            // The detail can name host paths, so it goes to the service log and not to the caller.
            eprintln!("soma-api: suspension store: {detail}");
            ApiError::new(
                500,
                "suspension_store_failed",
                "the suspension store refused the operation",
                true,
            )
        }
    }
}
