//! `GET /v1/capacity`: what this host has left for sandboxes, so a placer can see it.

use serde::Serialize;
use soma::SandboxPhase;
use soma_local::capacity::HostCapacity;

use super::support::success;
use crate::envelope::ApiError;
use crate::facade::SandboxFacade;
use crate::failure::managed_error;
use crate::http::response::Response;

/// The memory sizes the report says how many more of would fit.
const FIT_SIZES_MIB: [u64; 5] = [512, 1024, 2048, 4096, 8192];

#[derive(Serialize)]
struct CapacityReport {
    host: HostCapacity,
    /// Sandboxes this host holds that are not ending.
    running_sandboxes: usize,
    /// For each size, how many more sandboxes of it fit in the memory left above the reserve.
    fits: Vec<Fit>,
    /// Sandboxes not ending that no owner record names: every tenant reads them as absent until
    /// the platform records their owner, so a host roll refuses while this is not empty.
    ownerless_sandboxes: Vec<soma::InstanceId>,
}

#[derive(Serialize)]
struct Fit {
    memory_mib: u64,
    sandboxes: Option<u64>,
}

pub(super) fn report<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    operation: &'static str,
) -> Result<Response, ApiError> {
    let host = facade.capacity().ok_or_else(|| {
        ApiError::new(
            501,
            "capacity_unavailable",
            "this service has no host whose capacity it can read",
            false,
        )
    })?;
    let listing = facade.list().map_err(|failure| managed_error(&failure))?;
    let running_sandboxes = listing
        .entries()
        .iter()
        .filter(|entry| entry.phase() != SandboxPhase::Terminating)
        .count();
    let fits = FIT_SIZES_MIB
        .iter()
        .map(|&memory_mib| Fit {
            memory_mib,
            sandboxes: host.fits(memory_mib),
        })
        .collect();
    let ownerless_sandboxes = super::scope::ownerless(facade)?;
    Ok(success(
        200,
        operation,
        &CapacityReport {
            host,
            running_sandboxes,
            fits,
            ownerless_sandboxes,
        },
        None,
    ))
}
