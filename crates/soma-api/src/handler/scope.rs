//! Answers a request only for the tenant that owns the sandbox or volume it names (SOMA#222).
//!
//! Another tenant's sandbox answers exactly as one that does not exist, and a listing names only
//! the caller's own. An identity with no recorded owner is answered as before; see
//! [`crate::owners`].

use soma::{InstanceId, ManagedFailure, ManagedStateError};
use soma_hostd::FailureCode;

use crate::{
    envelope::ApiError,
    facade::SandboxFacade,
    failure::managed_error,
    http::response::Response,
    owners::{Access, Claim, Resource},
    report::SandboxListReport,
    route::Route,
    tenant::TenantId,
    volumes::VolumeFailure,
};

/// The existing sandbox a route acts on, if it acts on one.
const fn addressed(route: &Route) -> Option<&InstanceId> {
    match route {
        Route::GetSandbox(id)
        | Route::StopSandbox(id)
        | Route::PauseSandbox(id)
        | Route::ResumeSandbox(id)
        | Route::CheckpointSandbox(id)
        | Route::DiscardCheckpoint(id)
        | Route::RestoreSandbox(id)
        | Route::ForkSandbox(id)
        | Route::SuspendSandbox(id)
        | Route::Autosize(id, _)
        | Route::DestroySandbox(id)
        | Route::RunCommand(id)
        | Route::Filesystem(id, _)
        | Route::Terminal(id, _)
        | Route::Tunnel(id, _)
        | Route::Resources(id, _) => Some(id),
        _ => None,
    }
}

/// Refuses a route on a sandbox another tenant owns, as if that sandbox did not exist.
///
/// # Errors
///
/// Returns the absent sandbox's refusal, or a refusal when ownership cannot be read.
pub(crate) fn admit<F: SandboxFacade + ?Sized>(
    facade: &F,
    route: &Route,
    tenant: &TenantId,
) -> Result<(), ApiError> {
    addressed(route).map_or(Ok(()), |id| {
        require(facade, Resource::Sandbox, id.as_str(), tenant)
    })
}

/// Refuses an identity another tenant owns, as if it did not exist.
pub(super) fn require<F: SandboxFacade + ?Sized>(
    facade: &F,
    resource: Resource,
    identity: &str,
    tenant: &TenantId,
) -> Result<(), ApiError> {
    let Some(owners) = facade.owners() else {
        return Ok(());
    };
    match owners
        .access(resource, identity, tenant.as_str())
        .map_err(|_| unavailable())?
    {
        Access::Other => Err(absent(resource)),
        Access::Owner | Access::Unclaimed => Ok(()),
    }
}

/// Takes an identity for `tenant` before the operation that creates it runs.
///
/// Answers whether this request took it first, so the caller gives it back if the operation then
/// fails. An identity another tenant holds is refused as one already in use.
pub(super) fn claim<F: SandboxFacade + ?Sized>(
    facade: &F,
    resource: Resource,
    identity: &str,
    tenant: &TenantId,
) -> Result<bool, ApiError> {
    let Some(owners) = facade.owners() else {
        return Ok(false);
    };
    match owners
        .claim(resource, identity, tenant.as_str())
        .map_err(|_| unavailable())?
    {
        Claim::New => Ok(true),
        Claim::Kept => Ok(false),
        Claim::Refused => Err(match resource {
            Resource::Sandbox => managed_error(&ManagedFailure::State(
                ManagedStateError::MachineAlreadyExists,
            )),
            Resource::Volume => VolumeFailure::Refused(FailureCode::VolumeBusy as u16).refusal(),
        }),
    }
}

/// Gives back an identity a failed create took, so the identity is free again.
pub(super) fn release<F: SandboxFacade + ?Sized>(facade: &F, resource: Resource, identity: &str) {
    if let Some(owners) = facade.owners() {
        let _ignored = owners.forget(resource, identity);
    }
}

/// Lists the sandboxes on this host that `tenant` may see.
pub(super) fn list<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    operation: &'static str,
    tenant: &TenantId,
) -> Result<Response, ApiError> {
    let listing = facade.list().map_err(|failure| managed_error(&failure))?;
    let mut report = SandboxListReport::from_listing(&listing);
    if let Some(owners) = facade.owners() {
        let visible = |id: &InstanceId| {
            owners
                .access(Resource::Sandbox, id.as_str(), tenant.as_str())
                .map(|access| access != Access::Other)
                .map_err(|_| unavailable())
        };
        let mut kept = Vec::with_capacity(report.sandboxes.len());
        for entry in report.sandboxes {
            if visible(&entry.instance_id)? {
                kept.push(entry);
            }
        }
        report.sandboxes = kept;
        report.count = report.sandboxes.len();
        let mut unreadable = Vec::with_capacity(report.unreadable.len());
        for entry in report.unreadable {
            if visible(&entry.instance_id)? {
                unreadable.push(entry);
            }
        }
        report.unreadable = unreadable;
    }
    Ok(super::success(200, operation, &report, None))
}

fn absent(resource: Resource) -> ApiError {
    match resource {
        Resource::Sandbox => {
            managed_error(&ManagedFailure::State(ManagedStateError::MachineNotFound))
        }
        Resource::Volume => VolumeFailure::Refused(FailureCode::VolumeMissing as u16).refusal(),
    }
}

fn unavailable() -> ApiError {
    ApiError::new(
        503,
        "ownership_unavailable",
        "the owner of the named resource could not be read",
        true,
    )
}
