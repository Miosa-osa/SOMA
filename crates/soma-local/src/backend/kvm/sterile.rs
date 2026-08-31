//! The body of a sandbox thread that restores a machine before any Instance exists.
//!
//! A restore pays for mapping the captured memory, creating the VM, restoring the vCPU and the
//! five device models, routing interrupts, and starting the event loop. None of that work
//! depends on which Instance the machine will serve, so all of it can be paid before a request
//! arrives. What remains on the request path is the private disk head, the context identifier,
//! and the leased frame path, which are the authorities that belong to one Instance.
//!
//! The machine cannot be moved between threads: a running vCPU holds a process-wide handler
//! guard that is not `Send`, so the type that contains it is not `Send` either. A prepared
//! worker is therefore a parked thread that already owns its machine and waits for an
//! assignment, rather than a machine object handed from a builder thread to a request thread.
//! That is also the shape a worker process eventually takes, so nothing moves when it does.

use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use soma_guest::{HostLaunchMaterial, SecretFile};
use soma_kvm::x86_64::{SnapshotPaths, Sterile, SterileRequest, restore_sterile};

use super::session::{Network, Request, Response, SessionError};
use super::worker::{LaunchInputs, drive_restored, report};

/// What one prepared worker restores, and deliberately nothing more.
///
/// It names the immutable artifacts and the size the private head will have. It does not name a
/// head, an Instance, an operation, a context identifier, a frame path, a secret, or any launch
/// material, because a worker holding any of those before it is claimed would not be sterile.
pub(super) struct SterileSpec {
    /// The published snapshot directory.
    pub(super) snapshot: PathBuf,
    /// The immutable root every Instance of this Generation shares.
    pub(super) root: File,
    /// The capacity the private head will have once one is attached.
    pub(super) overlay_capacity_bytes: u64,
    /// Guest RAM in bytes.
    pub(super) memory_bytes: u64,
}

/// The fresh per-Instance authority a claim transfers into one prepared worker, exactly once.
pub(super) struct Assignment {
    /// This Instance's private writable head.
    pub(super) overlay: File,
    /// The Generation the launch material is bound to.
    pub(super) generation: [u8; 32],
    /// The Instance identity.
    pub(super) instance: [u8; 16],
    /// The Launch operation.
    pub(super) operation: [u8; 16],
    /// The vsock context identifier this Instance is assigned.
    pub(super) guest_cid: u32,
    /// The network this Instance was given: the launch values, the leased frame path the machine
    /// attaches, and what the repaired session must mint before traffic may flow.
    pub(super) network: Network,
    /// The secrets this one Instance is launched with.
    pub(super) secrets: Vec<SecretFile>,
}

/// Restores one sterile machine, parks until it is assigned, then serves it to its end.
///
/// Every path that does not reach an assignment drops the machine, which releases the VM, the
/// mapping, and every descriptor it owns. A sterile worker is single use: it is never returned
/// to the pool once this function has taken an assignment from the channel.
pub(super) fn serve(spec: SterileSpec, requests: &Receiver<Request>, responses: &Sender<Response>) {
    let SterileSpec {
        snapshot,
        root,
        overlay_capacity_bytes,
        memory_bytes,
    } = spec;
    let sterile = restore_sterile(SterileRequest {
        paths: SnapshotPaths::new(snapshot),
        root,
        overlay_capacity_bytes,
        memory_bytes,
        // Re-hashing every byte of the memory object is the installation and audit boundary,
        // not the preparation path.
        verify_artifacts: false,
    });
    let Ok(sterile) = sterile else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };
    // Announcing readiness to be claimed is the last thing that happens before the park. A
    // closed channel here means the pool went away while the machine was being built, so the
    // machine is dropped rather than left parked with no owner.
    if responses.send(Response::Prepared).is_err() {
        return;
    }

    // Anything other than an assignment ends the worker. A closed channel is the ordinary end:
    // the pool was dropped, or a claim was abandoned before it transferred authority.
    let Ok(Request::Assign(assignment)) = requests.recv() else {
        return;
    };
    assign(sterile, *assignment, requests, responses);
}

/// Gives one sterile machine its Instance authority and drives it to the end of its life.
fn assign(
    sterile: Sterile,
    assignment: Assignment,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) {
    let Assignment {
        overlay,
        generation,
        instance,
        operation,
        guest_cid,
        network,
        secrets,
    } = assignment;
    let Network {
        launch,
        attachment,
        activation,
    } = network;
    let Ok(material) = HostLaunchMaterial::generate(generation, instance, operation, launch) else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };
    // `assign` consumes the sterile machine and validates the head shape and the context
    // identifier before it commits any device mutation, so a refusal destroys the worker here
    // rather than returning a half-assigned one to anybody. The leased frame path is installed
    // in the same step, because it is this Instance's authority and not the pool's: a prepared
    // machine that already held a TAP would be holding one tenant's network.
    let Ok(mut restored) = sterile.assign(overlay, guest_cid, attachment) else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };
    let inputs = LaunchInputs {
        material,
        secrets: &secrets,
    };
    let outcome = drive_restored(
        &mut restored,
        inputs,
        (instance, operation, activation),
        requests,
        responses,
    );
    report(restored.machine, outcome, responses, instance);
}
