//! Reading a suspended machine into the three plaintext members of a Suspension (ADR 0034).
//!
//! The guest agent has already acknowledged Suspend: it took the link down, flushed the overlay,
//! and retired the session before it answered. What is left is the host's half, which is the same
//! quiesce and read a Generation capture performs, started from that acknowledgement instead of
//! from a console line. The members are written plaintext into the staging directory the owner
//! named; sealing them under the Suspension's data key and removing the plaintext is the store's
//! job, on the host's encrypted data mount.
//!
//! A captured machine is never resumed in place: it is released here, after the read, and a
//! resume restores a new machine from the sealed Suspension.

use std::fs::File;
use std::time::{Duration, Instant};

use soma_kvm::x86_64::{
    CaptureOutcome, CapturePoint, CaptureRequest, SandboxMachine, SnapshotPaths, capture,
};

use super::super::session::{SessionError, SuspendTarget};

/// The console line a suspended agent prints, recorded as the state member's marker so a
/// Suspension's state can never be mistaken for a Generation's.
pub const SUSPENSION_POINT_LINE: &[u8] = b"suspended, awaiting launch material";

/// How long each vCPU may take to leave `KVM_RUN` when the machine is paused for the read.
const PAUSE_GRACE: Duration = Duration::from_secs(5);
/// How long the whole read may take before it is abandoned.
const CAPTURE_CEILING: Duration = Duration::from_secs(600);

/// The disk handles a suspension reads, duplicated when the machine was built.
pub struct SuspendDisks {
    pub root: File,
    pub overlay: Option<File>,
}

/// Reads the paused machine into `target`.
///
/// # Errors
///
/// Returns [`SessionError::Suspend`] when the machine could not be quiesced or read; nothing
/// complete is published then.
pub fn read(
    machine: &mut SandboxMachine,
    disks: &mut SuspendDisks,
    target: &SuspendTarget,
    generation: [u8; 32],
    contract: soma_kvm::contract::MachineContract,
) -> Result<CaptureOutcome, SessionError> {
    let request = CaptureRequest {
        paths: SnapshotPaths::new(target.directory.clone()),
        candidate_id: generation,
        root: &mut disks.root,
        overlay: disks.overlay.as_mut(),
        repair_point_line: SUSPENSION_POINT_LINE.to_vec(),
        grace: PAUSE_GRACE,
        contract,
        point: CapturePoint::Acknowledged,
    };
    let started = Instant::now();
    let captured = capture(machine, request, Instant::now() + CAPTURE_CEILING);
    eprintln!(
        "soma: suspension capture took {} ms",
        started.elapsed().as_millis()
    );
    captured.map_err(|error| {
        // The typed answer carries no detail, so the reason is written where the machine host's
        // log goes (SOMA_MACHINE_HOST_LOG), for the operator who has to explain a lost machine.
        eprintln!("soma: suspension capture failed: {error}");
        SessionError::Suspend
    })
}
