use serde::{Deserialize, Serialize};

use super::{CleanupDisposition, cleanup_terminal};

/// Terminal cleanup evidence for the independently owned network resources of one Machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkCleanupEvidence {
    lease: CleanupDisposition,
    runtime_attachment: CleanupDisposition,
    address_leases: CleanupDisposition,
    egress_policy: CleanupDisposition,
    dns_policy: CleanupDisposition,
    proxy_policy: CleanupDisposition,
    ingress_bindings: CleanupDisposition,
}

impl NetworkCleanupEvidence {
    #[must_use]
    pub const fn new(
        lease: CleanupDisposition,
        runtime_attachment: CleanupDisposition,
        address_leases: CleanupDisposition,
        egress_policy: CleanupDisposition,
        dns_policy: CleanupDisposition,
        proxy_policy: CleanupDisposition,
        ingress_bindings: CleanupDisposition,
    ) -> Self {
        Self {
            lease,
            runtime_attachment,
            address_leases,
            egress_policy,
            dns_policy,
            proxy_policy,
            ingress_bindings,
        }
    }

    pub(super) const fn uniform(disposition: CleanupDisposition) -> Self {
        Self::new(
            disposition,
            disposition,
            disposition,
            disposition,
            disposition,
            disposition,
            disposition,
        )
    }

    #[must_use]
    pub const fn lease(&self) -> CleanupDisposition {
        self.lease
    }

    #[must_use]
    pub const fn runtime_attachment(&self) -> CleanupDisposition {
        self.runtime_attachment
    }

    #[must_use]
    pub const fn address_leases(&self) -> CleanupDisposition {
        self.address_leases
    }

    #[must_use]
    pub const fn egress_policy(&self) -> CleanupDisposition {
        self.egress_policy
    }

    #[must_use]
    pub const fn dns_policy(&self) -> CleanupDisposition {
        self.dns_policy
    }

    #[must_use]
    pub const fn proxy_policy(&self) -> CleanupDisposition {
        self.proxy_policy
    }

    #[must_use]
    pub const fn ingress_bindings(&self) -> CleanupDisposition {
        self.ingress_bindings
    }

    pub(super) const fn is_complete(&self) -> bool {
        cleanup_terminal(self.lease)
            && cleanup_terminal(self.runtime_attachment)
            && cleanup_terminal(self.address_leases)
            && cleanup_terminal(self.egress_policy)
            && cleanup_terminal(self.dns_policy)
            && cleanup_terminal(self.proxy_policy)
            && cleanup_terminal(self.ingress_bindings)
    }

    pub(super) const fn all_not_owned(&self) -> bool {
        matches!(self.lease, CleanupDisposition::NotOwned)
            && matches!(self.runtime_attachment, CleanupDisposition::NotOwned)
            && matches!(self.address_leases, CleanupDisposition::NotOwned)
            && matches!(self.egress_policy, CleanupDisposition::NotOwned)
            && matches!(self.dns_policy, CleanupDisposition::NotOwned)
            && matches!(self.proxy_policy, CleanupDisposition::NotOwned)
            && matches!(self.ingress_bindings, CleanupDisposition::NotOwned)
    }
}
