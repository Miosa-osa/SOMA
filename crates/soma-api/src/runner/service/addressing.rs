//! Which sandbox one path names, and which runner owns it.
//!
//! An id carries the tag of the host that minted it, so addressing is the one place that decides
//! whether a request is this runner's to serve or belongs to the runner the answer names.

use crate::runner::{
    ids::SandboxId,
    public_wire::{self, PlatformError},
};

use super::{Runner, RunnerResponse};

impl Runner {
    /// Parses a path id and checks that this runner owns its tag.
    pub(super) fn addressed(&self, raw_id: &str) -> Result<SandboxId, Box<RunnerResponse>> {
        let id = SandboxId::parse(raw_id)
            .ok_or_else(|| Box::new(RunnerResponse::platform(&PlatformError::invalid_id())))?;
        if id.tag() != self.config.host_tag {
            let runner_url = self.runner_url(id.tag());
            return Err(Box::new(RunnerResponse::new(
                421,
                public_wire::misdirected(&runner_url),
            )));
        }
        Ok(id)
    }

    /// `https://<tag>.<domain>`: where every call for a sandbox with this tag goes (C1).
    pub(super) fn runner_url(&self, tag: char) -> String {
        format!("https://{tag}.{}", self.config.public_domain)
    }
}
