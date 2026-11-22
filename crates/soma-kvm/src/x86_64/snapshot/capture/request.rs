//! What a capture is asked for, and what it publishes.

use std::fs::File;
use std::time::Instant;

use super::super::artifacts::SnapshotPaths;
use crate::snapshot::Digest;

/// How a capture learns that the guest agent is parked and the machine may be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturePoint {
    /// A Generation build: the agent prints the repair-point line on the watched console after
    /// booting the Generation, and the capture waits for it.
    ConsoleLine,
    /// A suspension (ADR 0034): the caller already holds the agent's authenticated Suspend
    /// acknowledgement, which the agent sends only after taking the link down and flushing the
    /// overlay. The session is retired with it, so the machine is read without a console wait.
    Acknowledged,
}

/// Everything a capture needs beyond the running machine itself.
pub struct CaptureRequest<'a> {
    /// Where the three objects are published.
    pub paths: SnapshotPaths,
    /// The identity of the Generation this machine was built from.
    pub candidate_id: [u8; 32],
    /// The immutable EROFS root, for its installation-time identity only.
    pub root: &'a mut File,
    /// The Instance-private overlay head, which becomes this snapshot's sterile template.
    ///
    /// A Generation with no writable storage has no head, publishes no `overlay.raw`, and every
    /// Instance restored from its snapshot therefore clones nothing.
    pub overlay: Option<&'a mut File>,
    /// The console line the agent prints at the repair point, recorded as the manifest's marker.
    pub repair_point_line: Vec<u8>,
    /// How the capture knows the agent is parked.
    pub point: CapturePoint,
    /// How long each vCPU may take to leave `KVM_RUN` after it is kicked.
    pub grace: std::time::Duration,
    /// The machine contract this Generation was built under, which the manifest binds.
    pub contract: crate::contract::MachineContract,
}

/// What one published snapshot is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureOutcome {
    pub paths: SnapshotPaths,
    pub memory_digest: Digest,
    pub memory_bytes: u64,
    pub overlay_digest: Digest,
    pub overlay_bytes: u64,
    pub state_digest: Digest,
    pub state_bytes: u64,
    /// Identity of the immutable root the restored Instances share.
    pub root_digest: Digest,
    /// When the agent announced the repair point, relative to the machine's own clock.
    pub repair_point_at: Instant,
    /// Receive buffers the driver had posted at the capture point: network, vsock, events.
    pub posted_buffers: [u32; 3],
}
