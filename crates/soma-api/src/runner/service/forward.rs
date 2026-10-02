//! The per-sandbox routes soma-api already serves on loopback, offered on the public runner
//! under the runner's own auth and tenant rule (contract C2).

use std::collections::HashSet;

use crate::{
    envelope::{Envelope, render},
    handler::handle,
    http::request::Request,
    report::SandboxListReport,
    runner::{ids::SandboxId, keys::Principal, public_wire::PlatformError, sandboxes::Unavailable},
    tenant::TENANT_HEADER,
};

use super::{Runner, RunnerResponse, Timing, routing::Forward};

/// The tenant label the loopback handler is given when the real tenant id does not fit its
/// identity grammar. The handler only checks the shape; ownership was decided by the runner.
const FALLBACK_TENANT_LABEL: &str = "runner";

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
        match self.sandboxes.owner_of(&id, &principal.key.tenant_id) {
            Ok(_) => {}
            Err(Unavailable::Busy) => {
                return RunnerResponse::platform(&PlatformError::exec_busy());
            }
            Err(Unavailable::NotFound | Unavailable::Destroyed(..)) => {
                return RunnerResponse::platform(&PlatformError::sandbox_not_found());
            }
        }
        let Some(instance_id) = id.instance_id() else {
            return RunnerResponse::platform(&PlatformError::sandbox_not_found());
        };
        let request = Request::from_parts(
            method,
            format!("/v1/sandboxes/{}{suffix}", instance_id.as_str()),
            vec![(
                TENANT_HEADER.to_owned(),
                tenant_label(&principal.key.tenant_id),
            )],
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
            Ok((Ok(entries), call)) => {
                timing.call = call;
                let entries: Vec<_> = entries
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

fn tenant_label(tenant_id: &str) -> String {
    let fits = !tenant_id.is_empty()
        && tenant_id.len() <= crate::tenant::MAX_TENANT_BYTES
        && tenant_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && tenant_id.starts_with(|first: char| first.is_ascii_alphanumeric())
        && tenant_id.ends_with(|last: char| last.is_ascii_alphanumeric());
    if fits {
        tenant_id.to_owned()
    } else {
        FALLBACK_TENANT_LABEL.to_owned()
    }
}
