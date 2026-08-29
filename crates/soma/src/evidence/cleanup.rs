use serde::{Deserialize, Serialize};

mod network;

pub use network::NetworkCleanupEvidence;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupDisposition {
    Complete,
    Incomplete,
    NotOwned,
    UnsupportedVerification,
}

/// How guest termination was achieved while releasing owned resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMethod {
    NotApplicable,
    Graceful,
    Forced,
    GracefulThenForced,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupEvidence {
    method: CleanupMethod,
    machine: CleanupDisposition,
    memory: CleanupDisposition,
    storage: CleanupDisposition,
    network: NetworkCleanupEvidence,
    guest_authority: CleanupDisposition,
}

impl CleanupEvidence {
    #[must_use]
    pub const fn new(
        machine: CleanupDisposition,
        memory: CleanupDisposition,
        storage: CleanupDisposition,
        network: CleanupDisposition,
        guest_authority: CleanupDisposition,
    ) -> Self {
        Self {
            method: CleanupMethod::Unavailable,
            machine,
            memory,
            storage,
            network: NetworkCleanupEvidence::uniform(network),
            guest_authority,
        }
    }

    #[must_use]
    pub const fn with_network(mut self, network: NetworkCleanupEvidence) -> Self {
        self.network = network;
        self
    }

    #[must_use]
    pub const fn with_method(mut self, method: CleanupMethod) -> Self {
        self.method = method;
        self
    }

    #[must_use]
    pub const fn complete_owned_machine() -> Self {
        Self::new(
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
        )
    }

    #[must_use]
    pub const fn not_owned() -> Self {
        Self::new(
            CleanupDisposition::NotOwned,
            CleanupDisposition::NotOwned,
            CleanupDisposition::NotOwned,
            CleanupDisposition::NotOwned,
            CleanupDisposition::NotOwned,
        )
        .with_method(CleanupMethod::NotApplicable)
    }

    #[must_use]
    pub const fn incomplete_owned_machine() -> Self {
        Self::new(
            CleanupDisposition::Incomplete,
            CleanupDisposition::Incomplete,
            CleanupDisposition::Incomplete,
            CleanupDisposition::Incomplete,
            CleanupDisposition::Incomplete,
        )
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        cleanup_terminal(self.machine)
            && cleanup_terminal(self.memory)
            && cleanup_terminal(self.storage)
            && self.network.is_complete()
            && cleanup_terminal(self.guest_authority)
    }

    #[must_use]
    pub const fn method(&self) -> CleanupMethod {
        self.method
    }

    #[must_use]
    pub const fn machine(&self) -> CleanupDisposition {
        self.machine
    }

    #[must_use]
    pub const fn memory(&self) -> CleanupDisposition {
        self.memory
    }

    #[must_use]
    pub const fn storage(&self) -> CleanupDisposition {
        self.storage
    }

    #[must_use]
    pub const fn network(&self) -> &NetworkCleanupEvidence {
        &self.network
    }

    #[must_use]
    pub const fn guest_authority(&self) -> CleanupDisposition {
        self.guest_authority
    }

    pub(crate) const fn all_not_owned(&self) -> bool {
        matches!(self.method, CleanupMethod::NotApplicable)
            && matches!(self.machine, CleanupDisposition::NotOwned)
            && matches!(self.memory, CleanupDisposition::NotOwned)
            && matches!(self.storage, CleanupDisposition::NotOwned)
            && self.network.all_not_owned()
            && matches!(self.guest_authority, CleanupDisposition::NotOwned)
    }
}

const fn cleanup_terminal(disposition: CleanupDisposition) -> bool {
    matches!(
        disposition,
        CleanupDisposition::Complete | CleanupDisposition::NotOwned
    )
}
