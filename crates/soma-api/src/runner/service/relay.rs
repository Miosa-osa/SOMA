//! Answering a call about another host's sandbox through its owner (contract C2).

use crate::runner::{ids::SandboxId, peers::Peers};

use super::{Runner, RunnerRequest, RunnerResponse, routing::Route};

impl Runner {
    /// Lets this runner forward calls about other hosts' sandboxes to their owners.
    pub fn set_peers(&mut self, peers: Peers) {
        self.peers = Some(peers);
    }

    /// Passes a call about another host's sandbox to its owner, adding `Soma-Runner-Url`.
    ///
    /// `None` means this runner answers itself: the id is ours or malformed, the request was
    /// already forwarded once, or the owner is unknown or unreachable (then `421`).
    pub(super) async fn forward_to_owner(
        &self,
        route: Route<'_>,
        request: &RunnerRequest,
    ) -> Option<RunnerResponse> {
        let tag = SandboxId::parse(route.sandbox_id()?)?.tag();
        if tag == self.config.host_tag || request.forwarded {
            return None;
        }
        let response = self.peers.as_ref()?.forward(tag, request).await?;
        let runner_url = self.runner_url(tag);
        Some(response.header("soma-runner-url", runner_url))
    }
}
