//! Answers a request only for the tenant that owns the sandbox or volume it names (SOMA#222).
//!
//! Another tenant's sandbox answers exactly as one that does not exist, and a listing names only
//! the caller's own. An identity with no recorded owner is answered only to the platform
//! ([`crate::tenant::PLATFORM_TENANT`]), which also answers for every tenant's; see
//! [`crate::owners`].

use soma::{InstanceId, ManagedFailure, ManagedStateError, SandboxPhase};
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
        Access::Owner => Ok(()),
        Access::Other | Access::Unclaimed if tenant.is_platform() => Ok(()),
        Access::Other | Access::Unclaimed => Err(absent(resource)),
    }
}

/// Takes a sandbox identity for `tenant` ahead of the create that names it, as the attach of its
/// volume does.
///
/// An identity no tenant owns whose sandbox is already on this host was written before owners
/// were recorded, so it is not handed to the first tenant that names it.
pub(super) fn reserve<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    identity: &str,
    tenant: &TenantId,
) -> Result<bool, ApiError> {
    let Some(owners) = facade.owners() else {
        return Ok(false);
    };
    let access = owners
        .access(Resource::Sandbox, identity, tenant.as_str())
        .map_err(|_| unavailable())?;
    match access {
        Access::Owner => Ok(false),
        Access::Other | Access::Unclaimed if tenant.is_platform() => Ok(false),
        Access::Other => Err(absent(Resource::Sandbox)),
        Access::Unclaimed => {
            let listing = facade.list().map_err(|failure| managed_error(&failure))?;
            let (sandboxes, unreadable) = listing.into_parts();
            let held = sandboxes
                .iter()
                .map(soma::SandboxEntry::instance_id)
                .chain(unreadable.iter().map(soma::UnreadableRecord::instance_id))
                .any(|held| held.as_str() == identity);
            if held {
                return Err(absent(Resource::Sandbox));
            }
            claim(facade, Resource::Sandbox, identity, tenant)
        }
    }
}

/// The sandboxes on this host that are not ending and that no owner record names.
///
/// Every tenant reads such a sandbox as absent, so each is named on the service's log when it
/// starts and in `GET /v1/capacity`, until the platform records its owner, rather than going
/// quiet as a 404 to the customer it belongs to.
///
/// # Errors
///
/// Returns the listing's refusal, or a refusal when an owner record cannot be read.
pub fn ownerless<F: SandboxFacade + ?Sized>(facade: &mut F) -> Result<Vec<InstanceId>, ApiError> {
    let listing = facade.list().map_err(|failure| managed_error(&failure))?;
    let Some(owners) = facade.owners() else {
        return Ok(Vec::new());
    };
    let mut ownerless = Vec::new();
    for entry in listing.entries() {
        if entry.phase() == SandboxPhase::Terminating {
            continue;
        }
        let id = entry.instance_id();
        if owners
            .owner(Resource::Sandbox, id.as_str())
            .map_err(|_| unavailable())?
            .is_none()
        {
            ownerless.push(id.clone());
        }
    }
    Ok(ownerless)
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
                .map(|access| access == Access::Owner || tenant.is_platform())
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
