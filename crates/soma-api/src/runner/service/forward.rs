//! The per-sandbox routes soma-api already serves on loopback, offered on the public runner
//! under the runner's own auth and tenant rule (contract C2).

use std::collections::HashSet;

use crate::{
    envelope::{Envelope, render},
    handler::handle,
    http::request::Request,
    report::SandboxListReport,
    runner::{ids::SandboxId, keys::Principal, public_wire::PlatformError, sandboxes::Unavailable},
    tenant::{PLATFORM_TENANT, TENANT_HEADER},
};

use super::{Runner, RunnerResponse, Timing, outcome, routing::Forward};

impl Runner {
    /// Serves one forwarded route for a sandbox the caller's tenant owns.
    ///
    /// The request handed to the loopback handler is built here from scratch: the method, the
    /// translated path, the body, and an `x-soma-tenant` header the runner writes from the key
    /// record. No header the client sent reaches it, so a client-supplied `x-soma-tenant` or
    /// any other trust header has nothing to act on.
    pub(super) async fn forward(
        &self,
        principal: &Principal,
        route: Forward<'_>,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let Forward {
            id: raw_id,
            method,
            suffix,
        } = route;
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        // An id whose sandbox answered its create before it was up is waited for here, so the
        // loopback handler behind this route never sees a sandbox that does not exist yet.
        let waited = match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => waited,
            Err(response) => return *response,
        };
        timing.wait = waited;
        // Held for the whole call, so a long terminal read never sees its sandbox reaped.
        let _held = match self.sandboxes.hold(&id, &principal.key.tenant_id) {
            Ok(held) => held,
            Err(refused) => return unavailable(&refused),
        };
        let Some(instance_id) = id.instance_id() else {
            return RunnerResponse::platform(&PlatformError::sandbox_not_found());
        };
        let request = Request::from_parts(
            method,
            format!("/v1/sandboxes/{}{suffix}", instance_id.as_str()),
            // The runner decided ownership above and keeps its own owners, so the loopback
            // handler is asked as the platform, which answers for a sandbox no tenant record names.
            vec![(TENANT_HEADER.to_owned(), PLATFORM_TENANT.to_owned())],
            body.to_vec(),
        );
        match self
            .backend
            .call(move |facade| handle(facade, &request))
            .await
        {
            Ok((response, call)) => {
                timing.call = call;
                RunnerResponse::new(response.status, response.body)
            }
            Err(_busy) => RunnerResponse::runtime_busy(),
        }
    }

    /// Lists the caller's tenant's sandboxes on this runner, in soma-api's list document.
    ///
    /// The facade lists every sandbox on the host; only the ones this tenant owns are kept.
    pub(super) async fn list(&self, principal: &Principal, timing: &mut Timing) -> RunnerResponse {
        let owned: HashSet<String> = self
            .sandboxes
            .owned_by(&principal.key.tenant_id)
            .iter()
            .filter_map(SandboxId::instance_id)
            .map(|instance| instance.as_str().to_owned())
            .collect();
        match self.backend.call(|facade| facade.list()).await {
            Ok((Ok(listing), call)) => {
                timing.call = call;
                // A record the store cannot decode has no owner this runner can check, so it is
                // not shown to a tenant; the operator is told it exists.
                if !listing.unreadable().is_empty() {
                    eprintln!(
                        "soma-api: runner listing skipped {} unreadable record(s): {}",
                        listing.unreadable().len(),
                        super::unreadable_ids(listing.unreadable())
                    );
                }
                let entries: Vec<_> = listing
                    .into_parts()
                    .0
                    .into_iter()
                    .filter(|entry| owned.contains(entry.instance_id().as_str()))
                    .collect();
                let report = SandboxListReport::new(&entries);
                RunnerResponse::new(
                    200,
                    render(&Envelope::success("sandbox.list", &report, None)),
                )
            }
            Ok((Err(_), call)) => {
                timing.call = call;
                RunnerResponse::platform(&PlatformError::agent_unavailable())
            }
            Err(_busy) => RunnerResponse::runtime_busy(),
        }
    }
}

/// The answer for a held call on a sandbox that cannot be held, shared by every held route.
pub(super) fn unavailable(refused: &Unavailable) -> RunnerResponse {
    match refused {
        Unavailable::Busy => RunnerResponse::platform(&PlatformError::exec_busy()),
        Unavailable::NotFound | Unavailable::Destroyed(..) => {
            RunnerResponse::platform(&PlatformError::sandbox_not_found())
        }
        // A launch that failed after its create had answered: the call is answered as the
        // create itself would have been. A call that arrived while the launch was still in
        // flight waited for it before it was held and read the same failure there.
        Unavailable::LaunchFailed(failure) => outcome::launch_failed(*failure),
    }
}
