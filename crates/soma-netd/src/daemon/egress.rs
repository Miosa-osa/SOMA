//! Changing the egress of an activated assignment on behalf of the peer that owns it.
//!
//! The checks are the assignment's own, the same ones a guest dial makes: the owner is the
//! authenticated peer that claimed the bundle and the Instance must be the one the ledger bound
//! to it. The assignment is taken out of the map for the change, as activation does, so a
//! concurrent request for the same bundle is told it is not assigned instead of racing a
//! half-applied ruleset.

use crate::{
    BundleId, CleanupGeneration, EgressClass, Error, InstanceId, PeerIdentity, Reply, error_code,
};

use super::{State, connect::authorize};

pub(super) fn set_egress(
    state: &State,
    peer: PeerIdentity,
    target: (InstanceId, BundleId, CleanupGeneration, EgressClass, bool),
) -> Reply {
    let (instance, bundle, generation, egress, dns) = target;
    let _busy = state.traffic.enter();
    let Some(mut owned) = state.disown((bundle, generation)) else {
        return Reply::Failed(error_code(&Error::NotAssigned));
    };
    if let Err(error) = authorize(owned.peer, peer.uid(), owned.assigned.record().instance, instance)
    {
        state.own(owned);
        return Reply::Failed(error_code(&error));
    }
    let reply = match state.broker.set_egress(&mut owned.assigned, egress, dns) {
        Ok(change) => Reply::EgressSet {
            bundle,
            generation,
            open: change.egress.forwards(),
            dns: change.dns,
            changed: change.changed,
        },
        Err(error) => Reply::Failed(error_code(&error)),
    };
    state.own(owned);
    reply
}
