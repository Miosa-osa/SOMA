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
    pub const fn uniform(disposition: CleanupDisposition) -> Self {
        Self {
            lease: disposition,
            runtime_attachment: disposition,
            address_leases: disposition,
            egress_policy: disposition,
            dns_policy: disposition,
            proxy_policy: disposition,
            ingress_bindings: disposition,
        }
    }

    #[must_use]
    pub const fn with_lease(mut self, disposition: CleanupDisposition) -> Self {
        self.lease = disposition;
        self
    }

    #[must_use]
    pub const fn with_runtime_attachment(mut self, disposition: CleanupDisposition) -> Self {
        self.runtime_attachment = disposition;
        self
    }

    #[must_use]
    pub const fn with_address_leases(mut self, disposition: CleanupDisposition) -> Self {
        self.address_leases = disposition;
        self
    }

    #[must_use]
    pub const fn with_egress_policy(mut self, disposition: CleanupDisposition) -> Self {
        self.egress_policy = disposition;
        self
    }

    #[must_use]
    pub const fn with_dns_policy(mut self, disposition: CleanupDisposition) -> Self {
        self.dns_policy = disposition;
        self
    }

    #[must_use]
    pub const fn with_proxy_policy(mut self, disposition: CleanupDisposition) -> Self {
        self.proxy_policy = disposition;
        self
    }

    #[must_use]
    pub const fn with_ingress_bindings(mut self, disposition: CleanupDisposition) -> Self {
        self.ingress_bindings = disposition;
        self
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_preserves_independent_resource_evidence() {
        let evidence = NetworkCleanupEvidence::uniform(CleanupDisposition::Complete)
            .with_lease(CleanupDisposition::NotOwned)
            .with_runtime_attachment(CleanupDisposition::Incomplete)
            .with_address_leases(CleanupDisposition::UnsupportedVerification)
            .with_egress_policy(CleanupDisposition::NotOwned)
            .with_dns_policy(CleanupDisposition::Incomplete)
            .with_proxy_policy(CleanupDisposition::UnsupportedVerification)
            .with_ingress_bindings(CleanupDisposition::NotOwned);

        assert_eq!(evidence.lease(), CleanupDisposition::NotOwned);
        assert_eq!(
            evidence.runtime_attachment(),
            CleanupDisposition::Incomplete
        );
        assert_eq!(
            evidence.address_leases(),
            CleanupDisposition::UnsupportedVerification
        );
        assert_eq!(evidence.egress_policy(), CleanupDisposition::NotOwned);
        assert_eq!(evidence.dns_policy(), CleanupDisposition::Incomplete);
        assert_eq!(
            evidence.proxy_policy(),
            CleanupDisposition::UnsupportedVerification
        );
        assert_eq!(evidence.ingress_bindings(), CleanupDisposition::NotOwned);
    }
}
