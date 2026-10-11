//! `GET /api/v1/sandboxes/{id}/tunnel/{port}`: a raw byte stream to a guest port (C9).
//!
//! The same key, tenant and rate rules as `exec` apply, because the route is dispatched after
//! them. Everything here happens before the `101`, so every refusal reaches the caller as an
//! ordinary status: an unknown or foreign sandbox is `404`, one owned by another runner is
//! `421` (an upgraded connection is never forwarded), a port outside the policy is `403`, a
//! full sandbox or host is `429`, a sandbox created without a network is `422`, and a guest that
//! does not answer is `502`.
//!
//! On success the response carries a [`TunnelHandoff`]: the guest connection, the place the
//! tunnel holds in the table, and the idle timeout. The transport sends the `101`, takes the
//! upgraded connection, and pumps bytes until either side closes or the tunnel idles out.

use std::time::Duration;

use crate::{
    runner::{keys::Principal, public_wire::PlatformError, sandboxes::Unavailable},
    tunnel::{Refusal, TunnelSlot, UPGRADE_TOKEN},
};

use super::{Runner, RunnerRequest, RunnerResponse, Timing};

/// A guest connection waiting for the transport to finish the upgrade.
pub struct TunnelHandoff {
    pub guest: tokio::net::UnixStream,
    /// Held until the tunnel ends, so the table counts it for exactly as long as it lives.
    pub slot: TunnelSlot,
    pub idle: Duration,
    /// Reports traffic as sandbox activity.
    pub touch: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for TunnelHandoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelHandoff")
            .field("idle", &self.idle)
            .finish_non_exhaustive()
    }
}

impl Runner {
    pub(super) async fn tunnel(
        &self,
        principal: &Principal,
        raw_id: &str,
        raw_port: &str,
        request: &RunnerRequest,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        let Some(port) = raw_port.parse::<u16>().ok().filter(|port| *port != 0) else {
            return RunnerResponse::platform(&PlatformError::new(
                400,
                "INVALID_PORT",
                "the tunnel port must be 1 to 65535",
                false,
            ));
        };
        if !request.upgrade {
            return RunnerResponse::platform(&PlatformError::new(
                426,
                "UPGRADE_REQUIRED",
                "a tunnel needs HTTP/1.1 with `Connection: Upgrade` and `Upgrade: soma-tunnel`",
                false,
            ))
            .header("upgrade", UPGRADE_TOKEN.to_owned())
            .header("connection", "Upgrade".to_owned());
        }
        // Held while the guest is dialled; the tunnel itself keeps its sandbox alive by
        // reporting traffic, so the sandbox's own lifetime still ends it.
        let _held = match self.sandboxes.hold(&id, &principal.key.tenant_id) {
            Ok(held) => held,
            Err(Unavailable::Busy) => {
                return RunnerResponse::platform(&PlatformError::exec_busy());
            }
            Err(Unavailable::NotFound | Unavailable::Destroyed(..)) => {
                return RunnerResponse::platform(&PlatformError::sandbox_not_found());
            }
        };
        let Some(instance_id) = id.instance_id() else {
            return RunnerResponse::platform(&PlatformError::sandbox_not_found());
        };
        let slot = match self.tunnels.admit(&id.to_string(), port) {
            Ok(slot) => slot,
            Err(refused) => return refusal(Refusal::from(refused)),
        };
        let dialled = {
            let instance_id = instance_id.clone();
            self.backend
                .call(move |facade| facade.tunnel(&instance_id, port))
                .await
        };
        match dialled {
            Ok((Ok(stream), call)) => {
                timing.call = call;
                let guest = stream
                    .set_nonblocking(true)
                    .and_then(|()| tokio::net::UnixStream::from_std(stream));
                let Ok(guest) = guest else {
                    return refusal(crate::facade::TunnelFailure::Unavailable.into());
                };
                let (sandboxes, touched) = (std::sync::Arc::clone(&self.sandboxes), id);
                let mut response = RunnerResponse::new(101, Vec::new());
                response.tunnel = Some(TunnelHandoff {
                    guest,
                    slot,
                    idle: self.tunnels.policy().idle,
                    touch: Box::new(move || sandboxes.touch(&touched)),
                });
                response
            }
            Ok((Err(failure), call)) => {
                timing.call = call;
                refusal(failure.into())
            }
            Err(_busy) => RunnerResponse::runtime_busy(),
        }
    }
}

fn refusal(refusal: Refusal) -> RunnerResponse {
    RunnerResponse::platform(&PlatformError::new(
        refusal.status,
        refusal.public_code,
        refusal.message,
        refusal.retryable,
    ))
}
