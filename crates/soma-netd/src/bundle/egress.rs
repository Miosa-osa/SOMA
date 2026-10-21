//! Changing the egress of one activated bundle.
//!
//! A bundle's outbound policy lives in its sandbox ruleset, rendered from the admitted intent.
//! [`Broker::set_egress`] renders that ruleset again for another egress class and DNS state and
//! swaps it in one `nft` transaction, so there is no instant at which the table is missing or
//! half written.
//!
//! Closing is stricter than opening. The new ruleset no longer accepts established flows, so
//! they stop at once, but their conntrack entries would still be there to match a later reopen.
//! So a close also flushes the sandbox namespace's entries and the bundle's host zone. A close
//! that fails after the swap has already denied traffic and still reports the failure, and
//! asking again repeats the flushes: a close is never reported as done on a retry that skipped
//! them.

use crate::{Assigned, Broker, EgressClass, Error, SandboxRuleset, nft};

/// What one egress change did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EgressChange {
    /// The egress class now in force.
    pub egress: EgressClass,
    /// Whether DNS is now permitted.
    pub dns: bool,
    /// Whether the ruleset differs from before the call.
    pub changed: bool,
}

impl Broker {
    /// Sets the egress class and DNS state of an activated assignment.
    ///
    /// Idempotent: asking for the state already in force answers `changed: false` without
    /// touching the kernel, except that closing always re-applies and re-flushes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidState`] for a bundle that is not activated, [`Error::InvalidIntent`]
    /// for DNS without egress, [`Error::InvalidState`] when DNS is asked for but the profile names
    /// no resolver, and the first kernel or tool failure otherwise. After a failed open the
    /// previous ruleset is still in force; after a failed close the ruleset already denies.
    pub fn set_egress(
        &self,
        assigned: &mut Assigned,
        egress: EgressClass,
        dns: bool,
    ) -> Result<EgressChange, Error> {
        if !assigned.active {
            return Err(Error::InvalidState("egress before activation"));
        }
        let resolvers = if dns {
            let resolvers = self.profile.resolvers().to_vec();
            if resolvers.is_empty() {
                return Err(Error::InvalidState("no resolver to allow"));
            }
            resolvers
        } else {
            Vec::new()
        };
        let next = assigned.current.with_egress(egress, resolvers)?;
        let closing = !next.egress().forwards();
        let changed = next != assigned.current;
        if !changed && !closing {
            return Ok(EgressChange {
                egress,
                dns,
                changed: false,
            });
        }
        let ruleset = SandboxRuleset {
            names: &assigned.bundle.names,
            lease: assigned.bundle.leases.guest,
            guest_mac: assigned.bundle.macs.guest,
            intent: &next,
            protected: self.profile.protected(),
            published: &assigned.published,
            connect_ports: self.connect_ports.ranges(),
        }
        .render();
        assigned
            .bundle
            .namespace
            .within(move || nft::apply(&ruleset))?;
        assigned.current = next;
        if closing {
            assigned.bundle.namespace.within(nft::flush_all)?;
            nft::flush_zone(assigned.bundle.zone)?;
        }
        Ok(EgressChange {
            egress,
            dns,
            changed,
        })
    }
}
