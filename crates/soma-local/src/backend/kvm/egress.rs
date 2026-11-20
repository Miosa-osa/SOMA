//! Opening and closing the egress of the windowed machine this process holds.

use soma::{InstanceId, NetworkPolicy};

use super::{KvmBackend, evidence::effective_network, network::Egress};
use crate::{EgressSetFailure, EgressSetOutcome, EgressSetRequest};

impl KvmBackend {
    /// Applies `request` to the sandbox this process is driving.
    ///
    /// Only a sandbox launched windowed holds a bundle meant to be changed, so a machine that
    /// merely has public egress, or none, is refused rather than altered. The network the machine
    /// reports about itself follows the change, so an inspection never describes a window that is
    /// closed as open, or the reverse.
    pub(super) fn set_egress_resident(
        &mut self,
        instance: &InstanceId,
        request: EgressSetRequest,
    ) -> Result<EgressSetOutcome, EgressSetFailure> {
        let Some(live) = self.live_for(instance) else {
            return Err(EgressSetFailure::NotFound);
        };
        if !live.windowed {
            return Err(EgressSetFailure::NotWindowed);
        }
        let Egress::Leased(lease) = &mut live.egress else {
            return Err(EgressSetFailure::NotWindowed);
        };
        let outcome = lease.set_egress(request)?;
        live.network = effective_network(&live.egress, &policy(outcome));
        Ok(outcome)
    }
}

/// The policy the broker now enforces, in the portable vocabulary.
fn policy(outcome: EgressSetOutcome) -> NetworkPolicy {
    use soma::{DnsPolicy, EgressPolicy};
    let (egress, dns) = match (outcome.open, outcome.dns) {
        (false, _) => (EgressPolicy::Denied, DnsPolicy::Denied),
        (true, true) => (EgressPolicy::PublicInternet, DnsPolicy::System),
        (true, false) => (EgressPolicy::PublicInternet, DnsPolicy::Denied),
    };
    NetworkPolicy::new(egress, dns, Vec::new()).unwrap_or_else(|_| NetworkPolicy::isolated())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reported_policy_follows_the_window() {
        use soma::{DnsPolicy, EgressPolicy};
        let closed = policy(EgressSetOutcome {
            open: false,
            dns: false,
            changed: true,
        });
        assert_eq!(closed.egress(), EgressPolicy::Denied);
        assert_eq!(closed.dns(), &DnsPolicy::Denied);

        let open = policy(EgressSetOutcome {
            open: true,
            dns: true,
            changed: true,
        });
        assert_eq!(open.egress(), EgressPolicy::PublicInternet);
        assert_eq!(open.dns(), &DnsPolicy::System);

        // Open without DNS must report no DNS, never the default resolver.
        let open_without_dns = policy(EgressSetOutcome {
            open: true,
            dns: false,
            changed: false,
        });
        assert_eq!(open_without_dns.dns(), &DnsPolicy::Denied);
    }
}
