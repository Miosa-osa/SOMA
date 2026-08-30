//! Claiming one bundle, and replaying that exact claim after an uncertain delivery.
//!
//! A claim is identified by the Instance and Launch operation it names, so a peer that lost
//! its reply, or was disconnected before it arrived, replays the same operation and receives
//! the same bundle, the same launch values, the same unspent activation challenge, and another
//! copy of the same TAP descriptor.
//! A replay that changes any bound field is a mismatch rather than a second lease, and a
//! replay from another peer identity is refused, so one operation can never hold two network
//! identities.

use std::{
    fs::File,
    io::Read,
    os::fd::{AsFd, OwnedFd},
};

use crate::{
    Assigned, BundleId, Error, InstanceId, NetworkIntent, OperationId, PeerIdentity, Reply, Step,
    bundle::AssignFailure, error_code, release, release_sterile, send_tap,
};

use super::{Owned, State};

/// The identities one claim binds: Instance, Launch operation, vsock CID, and admitted intent.
pub(super) type ClaimRequest<'a> = (InstanceId, OperationId, u32, &'a NetworkIntent);

pub(super) fn claim(
    state: &mut State,
    connection: &OwnedFd,
    peer: PeerIdentity,
    request: ClaimRequest<'_>,
) -> Reply {
    let (instance, operation, vsock_cid, intent) = request;
    if state.operations.contains_key(&(instance, operation)) {
        return replay(state, connection, peer, request);
    }
    let bundle = match state.pool.pop_front() {
        Some(bundle) => bundle,
        None => match fresh_id().and_then(|id| state.broker.prepare(id)) {
            Ok(bundle) => bundle,
            Err(error) => return Reply::Failed(error_code(&error)),
        },
    };
    let assigned = match state
        .broker
        .assign(bundle, instance, operation, intent, vsock_cid)
    {
        Ok(assigned) => assigned,
        Err(AssignFailure { bundle, error }) => {
            drop(release_sterile(&state.broker, *bundle, Vec::new()));
            return Reply::Failed(error_code(&error));
        }
    };
    match deliver_descriptor(connection, &assigned) {
        Ok(reply) => {
            state.own(Owned {
                peer: peer.uid(),
                assigned,
            });
            reply
        }
        Err(error) => {
            drop(release(&state.broker, assigned));
            Reply::Failed(error_code(&error))
        }
    }
}

/// Answers a claim whose Instance and operation this broker already assigned.
fn replay(
    state: &mut State,
    connection: &OwnedFd,
    peer: PeerIdentity,
    request: ClaimRequest<'_>,
) -> Reply {
    let (instance, operation, vsock_cid, intent) = request;
    let Some(key) = state.operations.get(&(instance, operation)).copied() else {
        return Reply::Failed(error_code(&Error::NotAssigned));
    };
    let Some(owned) = state.assigned.get(&key) else {
        return Reply::Failed(error_code(&Error::NotAssigned));
    };
    if owned.peer != peer.uid() {
        return Reply::Failed(error_code(&Error::Unauthorized("assignment owner")));
    }
    let record = owned.assigned.record();
    if record.vsock_cid != vsock_cid || record.intent_digest != intent.digest() {
        return Reply::Failed(error_code(&Error::ReplayMismatch));
    }
    match deliver_descriptor(connection, &owned.assigned) {
        Ok(reply) => reply,
        Err(error) => Reply::Failed(error_code(&error)),
    }
}

/// Transfers the TAP descriptor and builds the exact reply that names it.
fn deliver_descriptor(connection: &OwnedFd, assigned: &Assigned) -> Result<Reply, Error> {
    let record = assigned.record();
    let header = crate::TransferHeader {
        bundle: record.bundle,
        generation: record.generation,
        intent: record.intent_digest,
    };
    let activation = assigned
        .activation_challenge()
        .cloned()
        .ok_or(Error::InvalidState("activation"))?;
    send_tap(connection.as_fd(), &header, assigned.bundle().tap().as_fd())?;
    Ok(Reply::Claimed {
        bundle: record.bundle,
        generation: record.generation,
        launch: launch_bytes(assigned),
        activation,
    })
}

fn launch_bytes(assigned: &Assigned) -> [u8; 35] {
    let launch = assigned.launch();
    let mut out = [0; 35];
    out[..4].copy_from_slice(&launch.vsock_cid().to_be_bytes());
    out[4..8].copy_from_slice(&launch.generation().to_be_bytes());
    out[8..14].copy_from_slice(&launch.mac());
    out[14..18].copy_from_slice(&launch.address());
    out[18] = launch.prefix_length();
    out[19..23].copy_from_slice(&launch.gateway());
    out[23..27].copy_from_slice(&launch.resolver());
    out[27..35].copy_from_slice(&launch.time_sample_nanos().to_be_bytes());
    out
}

pub(super) fn fresh_id() -> Result<BundleId, Error> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| Error::io(Step::OpenTun, &error))?;
    BundleId::new(bytes)
}
