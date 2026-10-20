//! Dialing one guest port on behalf of the peer that owns the assignment.
//!
//! Every check that makes this safe is a property of the assignment the broker already holds,
//! not of anything the request says: the owner is the authenticated peer that claimed the
//! bundle, the Instance must be the one the ledger bound to it, the destination is that
//! bundle's own guest, and the port must be on the broker's allowlist.

use std::os::fd::{AsFd, OwnedFd};

use crate::{
    BundleId, CleanupGeneration, Error, InstanceId, PeerIdentity, Reply, TransferHeader,
    error_code, send_tap,
};

use super::State;

pub(super) fn connect(
    state: &State,
    connection: &OwnedFd,
    peer: PeerIdentity,
    target: (InstanceId, BundleId, CleanupGeneration, u16),
) -> Reply {
    let (instance, bundle, generation, port) = target;
    // Looked up and checked under the lock; the dial itself, which can wait for the guest,
    // runs after it is released so a slow guest never delays a claim.
    let (plan, header) = {
        let maps = state.maps();
        let Some(owned) = maps.assigned.get(&(bundle, generation)) else {
            return Reply::Failed(error_code(&Error::NotAssigned));
        };
        let record = owned.assigned.record();
        if let Err(error) = authorize(owned.peer, peer.uid(), record.instance, instance) {
            return Reply::Failed(error_code(&error));
        }
        let plan = match state.broker.connect_plan(&owned.assigned, port) {
            Ok(plan) => plan,
            Err(error) => return Reply::Failed(error_code(&error)),
        };
        let header = TransferHeader {
            bundle: record.bundle,
            generation: record.generation,
            intent: record.intent_digest,
        };
        (plan, header)
    };
    let socket = match plan.dial() {
        Ok(socket) => socket,
        Err(error) => return Reply::Failed(error_code(&error)),
    };
    match send_tap(connection.as_fd(), &header, socket.as_fd()) {
        Ok(()) => Reply::Connected { bundle, generation },
        Err(error) => Reply::Failed(error_code(&error)),
    }
}

/// Whether this peer may dial the guest of an assignment claimed by `owner` for `bound`.
///
/// The peer is the kernel-derived identity of the connection and the Instance is what the
/// request names; both must agree with the assignment the broker holds.
pub(super) fn authorize(owner: u32, peer: u32, bound: InstanceId, asked: InstanceId) -> Result<(), Error> {
    if owner != peer {
        return Err(Error::Unauthorized("assignment owner"));
    }
    if bound != asked {
        return Err(Error::Unauthorized("assignment instance"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(byte: u8) -> InstanceId {
        InstanceId::new([byte; 16]).expect("id")
    }

    #[test]
    fn only_the_claiming_peer_and_the_bound_instance_may_dial() {
        assert_eq!(authorize(7, 7, instance(1), instance(1)), Ok(()));
        assert_eq!(
            authorize(7, 8, instance(1), instance(1)),
            Err(Error::Unauthorized("assignment owner"))
        );
        assert_eq!(
            authorize(7, 7, instance(1), instance(2)),
            Err(Error::Unauthorized("assignment instance"))
        );
        // A wrong peer is reported as such even when the instance is also wrong, so the reply
        // never confirms which instance a bundle belongs to.
        assert_eq!(
            authorize(7, 8, instance(1), instance(2)),
            Err(Error::Unauthorized("assignment owner"))
        );
    }
}
