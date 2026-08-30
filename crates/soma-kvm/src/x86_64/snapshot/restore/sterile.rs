//! A restored machine that does not yet serve any Instance.
//!
//! A Launch pays for machine creation on the request path, and a prepared worker exists to have
//! paid it already. That requires a machine which can be built before the Instance it will serve,
//! which means building it without the two things that belong to that Instance: the private disk
//! head and the vsock context identifier.

use std::fs::File;

use super::{
    Cell, Digest, RestoreFacts, RestoreSequence, Restored, SandboxMachine, SnapshotError,
    SnapshotPaths, readiness,
};

/// What a prepared worker is restored from, before any Instance exists.
///
/// It names the immutable artifacts and the shape of the private head, and deliberately not the
/// head itself, the context identifier, or the launch page. Those are per-Instance authority
/// that the prepared worker protocol transfers when the worker is claimed, and a worker holding
/// any of them before then would not be sterile.
pub struct SterileRequest {
    /// The published snapshot directory.
    pub paths: SnapshotPaths,
    /// The immutable root, which every Instance of this Generation shares.
    pub root: File,
    /// The capacity the private head will have when one is attached.
    pub overlay_capacity_bytes: u64,
    /// Guest RAM the caller expects, from the Generation shape rather than from the snapshot.
    pub memory_bytes: u64,
    /// Whether to re-hash the memory object and the overlay template before mapping.
    pub verify_artifacts: bool,
}

/// A restored machine that holds no per-Instance authority yet.
///
/// It has paid everything a restore costs except the last two steps, so a pool of these is what
/// lets a Launch skip machine creation entirely.
pub struct Sterile {
    pub(super) machine: SandboxMachine,
    pub(super) facts: SterileFacts,
    pub(super) sequence: RestoreSequence,
}

/// What the snapshot said this machine is, before an Instance is assigned to it.
pub(super) struct SterileFacts {
    pub(super) snapshot: Digest,
    pub(super) candidate_id: [u8; 32],
    pub(super) memory_bytes: u64,
    pub(super) repair_point_line: Vec<u8>,
    pub(super) mac: [u8; 6],
    pub(super) captured_cid: u64,
}

impl Sterile {
    /// Consumes the sterile worker and gives the resulting restore fresh Instance authority.
    ///
    /// The readiness secret is sampled only after resource assignment succeeds.
    /// Any failure consumes and drops the machine instead of returning partial authority.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for an invalid private head, CID, or readiness secret.
    pub fn assign(self, overlay: File, guest_cid: u32) -> Result<Restored, SnapshotError> {
        let Self {
            machine,
            facts,
            sequence,
        } = self;
        machine.assign_instance_resources(overlay, guest_cid)?;
        let readiness = readiness::sample_challenge()?;
        Ok(Restored {
            machine,
            facts: RestoreFacts {
                snapshot: facts.snapshot,
                candidate_id: facts.candidate_id,
                memory_bytes: facts.memory_bytes,
                repair_point_line: facts.repair_point_line,
                mac: facts.mac,
                captured_cid: facts.captured_cid,
                guest_cid,
            },
            sequence: Cell::new(sequence),
            readiness,
            spent: Cell::new(false),
            launch: Cell::new(None),
        })
    }
}
