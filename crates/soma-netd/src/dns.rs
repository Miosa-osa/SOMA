//! Resolver policy: the single launch-page resolver, resolver file evidence, and the
//! addresses the firewall may admit on port 53.

use std::net::Ipv4Addr;

use crate::NetworkIntent;

/// The resolver plan of one assigned bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DnsPlan {
    launch_resolver: Ipv4Addr,
    allowed: Vec<Ipv4Addr>,
}

impl DnsPlan {
    /// Derives the plan from the admitted intent and the bundle gateway.
    ///
    /// The launch page carries exactly one IPv4 resolver.
    /// When DNS is permitted that is the first declared resolver; when DNS is denied the
    /// gateway address is delivered so the guest never learns an operator resolver and every
    /// port 53 packet is dropped by the ruleset.
    #[must_use]
    pub fn from_intent(intent: &NetworkIntent, gateway: Ipv4Addr) -> Self {
        let allowed = intent.resolvers().to_vec();
        let launch_resolver = allowed.first().copied().unwrap_or(gateway);
        Self {
            launch_resolver,
            allowed,
        }
    }

    /// Returns the resolver delivered in the launch page.
    #[must_use]
    pub const fn launch_resolver(&self) -> Ipv4Addr {
        self.launch_resolver
    }

    /// Returns the resolvers the firewall admits on port 53; empty denies DNS.
    #[must_use]
    pub fn allowed(&self) -> &[Ipv4Addr] {
        &self.allowed
    }

    /// Renders the resolver file the guest agent will write from the same launch resolver.
    #[must_use]
    pub fn resolv_conf(&self) -> String {
        format!(
            "nameserver {}\noptions timeout:2 attempts:2\n",
            self.launch_resolver
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EgressClass, ProfileDigest};

    #[test]
    fn denied_dns_delivers_the_gateway_and_admits_nothing() {
        let intent = NetworkIntent::new(
            EgressClass::PublicInternet,
            Vec::new(),
            Vec::new(),
            ProfileDigest([1; 32]),
        )
        .expect("intent");
        let plan = DnsPlan::from_intent(&intent, Ipv4Addr::new(10, 200, 0, 1));
        assert_eq!(plan.launch_resolver(), Ipv4Addr::new(10, 200, 0, 1));
        assert!(plan.allowed().is_empty());
        assert_eq!(
            plan.resolv_conf(),
            "nameserver 10.200.0.1\noptions timeout:2 attempts:2\n"
        );
    }

    #[test]
    fn declared_resolvers_are_delivered_first_and_admitted_all() {
        let intent = NetworkIntent::new(
            EgressClass::PublicInternet,
            vec![Ipv4Addr::new(9, 9, 9, 9), Ipv4Addr::new(1, 1, 1, 1)],
            Vec::new(),
            ProfileDigest([1; 32]),
        )
        .expect("intent");
        let plan = DnsPlan::from_intent(&intent, Ipv4Addr::new(10, 200, 0, 1));
        assert_eq!(plan.launch_resolver(), Ipv4Addr::new(9, 9, 9, 9));
        assert_eq!(plan.allowed().len(), 2);
    }
}
