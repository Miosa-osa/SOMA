//! Commit and abort of a checkpoint received from off-host storage (ADR 0070).
//!
//! The raw export and receive routes are served by the connection (`crate::transfer`); these two
//! carry small JSON bodies and run like any other handler, platform tenant only.

use super::{ApiError, Response, SandboxFacade, parse, success};
use crate::{route::TransferRoute, tenant::TenantId, transfer::refusal};

/// What a commit publishes: the capture's owner and full receipt, and whether it reserves an
/// Instance this host does not hold (ADR 0066) rather than serving the one it does.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommitBody {
    owner: soma::MachineName,
    receipt: soma_local::CheckpointReceipt,
    #[serde(default)]
    absent: bool,
}

#[derive(serde::Serialize)]
struct Published<'a> {
    import_id: &'a str,
    instance_id: &'a str,
    checkpoint_id: &'a str,
    state: &'static str,
}

pub(super) fn serve<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    route: &TransferRoute,
    body: &[u8],
    tenant: &TenantId,
) -> Result<Response, ApiError> {
    if !tenant.is_platform() {
        return Err(ApiError::not_found());
    }
    match route {
        TransferRoute::Commit(import) => commit(facade, import, body, route.operation()),
        TransferRoute::Abort(import) => abort(import, route.operation()),
        // Both stream their bodies and are answered by the connection before any handler runs.
        TransferRoute::Export | TransferRoute::Receive(..) => Err(ApiError::not_found()),
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn commit<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    import: &soma::OperationId,
    body: &[u8],
    operation: &'static str,
) -> Result<Response, ApiError> {
    let body: CommitBody = parse(body)?;
    let stage = soma_local::ImportStage::open(import).map_err(refusal)?;
    facade
        .commit_checkpoint_import(&body.owner, &body.receipt, &stage, body.absent)
        .map_err(refusal)?;
    Ok(success(
        200,
        operation,
        &Published {
            import_id: import.as_str(),
            instance_id: body.receipt.instance_id.as_str(),
            checkpoint_id: body.receipt.checkpoint_id.as_str(),
            state: "published",
        },
        None,
    ))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn abort(import: &soma::OperationId, operation: &'static str) -> Result<Response, ApiError> {
    soma_local::ImportStage::open(import)
        .and_then(soma_local::ImportStage::abort)
        .map_err(refusal)?;
    Ok(success(
        200,
        operation,
        &serde_json::json!({"import_id": import.as_str(), "state": "absent"}),
        None,
    ))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn commit<F: SandboxFacade + ?Sized>(
    _facade: &mut F,
    _import: &soma::OperationId,
    body: &[u8],
    _operation: &'static str,
) -> Result<Response, ApiError> {
    let _body: CommitBody = parse(body)?;
    Err(refusal(soma_local::CheckpointFailure::Unsupported))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn abort(_import: &soma::OperationId, _operation: &'static str) -> Result<Response, ApiError> {
    Err(refusal(soma_local::CheckpointFailure::Unsupported))
}
