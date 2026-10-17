//! One sandbox's record: the phase it is in, and the little that may happen to a record.
//!
//! Beside this module is the collection that holds these. What is here is the type itself, the
//! guard a call takes while it uses a sandbox, and the three operations that only ever move a
//! record or take it away; they are together because they all read the same fields, and apart
//! from the collection because the collection is about serving calls, not about a record.

use std::time::{Duration, Instant};

use crate::runner::{
    idle::Clock,
    ids::SandboxId,
    journal::ExpireReason,
    owner::{LaunchFailure, Owner},
    starting::Starting,
};

use super::Sandboxes;

/// One call holding a sandbox (see [`Sandboxes::hold`]); dropping it lets the sweep back in.
pub struct Held<'a> {
    pub(super) sandboxes: &'a Sandboxes,
    pub(super) id: SandboxId,
    pub owner: Owner,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(record) = self.sandboxes.lock().get_mut(&self.id) {
            record.in_flight = record.in_flight.saturating_sub(1);
            record.clock.touch(Instant::now());
        }
    }
}

/// How long a destroyed sandbox is remembered, so a repeated destroy answers as the first did.
pub const TOMBSTONE_RETENTION: Duration = Duration::from_mins(10);

#[derive(Clone, Debug)]
pub(super) enum Phase {
    /// Reserved by a create that has not finished.
    Creating,
    /// A create answered `201` before its launch finished; the launch is still in flight, and
    /// every call on this sandbox waits for it before it proceeds.
    Starting(Starting),
    Ready,
    /// One command or destroy is in flight.
    Busy,
    /// A launch that failed after its create had answered. The sandbox was never created, so no
    /// call on it can proceed; the record is kept for as long as a destroyed one so the create's
    /// own failure stays readable instead of becoming a `404` that hides it.
    Failed {
        failure: LaunchFailure,
        at: Instant,
    },
    Destroyed {
        at: Instant,
    },
}

impl Phase {
    /// Whether this record is a sandbox the host holds and can still be commanded.
    ///
    /// A sandbox whose launch is still in flight is one of those: it holds a machine, a tenant
    /// slot and an id, and every call on it waits for it rather than being turned away.
    pub(super) const fn live(&self) -> bool {
        matches!(
            self,
            Self::Creating | Self::Starting(_) | Self::Ready | Self::Busy
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct Record {
    pub(super) owner: Owner,
    pub(super) clock: Clock,
    pub(super) phase: Phase,
    /// Calls that hold the sandbox without its lifecycle slot (inspect, files, terminal).
    /// They overlap freely with each other and with a command; the sweep waits for all.
    pub(super) in_flight: u32,
}

impl Sandboxes {
    /// Records that a sandbox is gone, giving its tenant slot back exactly once.
    ///
    /// A failed launch has already given the slot back, so its tombstone is only re-dated here:
    /// the slot is returned once per sandbox however many of these run.
    pub fn destroyed(&self, id: &SandboxId, at: Instant) {
        if let Some(record) = self.lock().get_mut(id)
            && !matches!(record.phase, Phase::Destroyed { .. })
        {
            let failed = matches!(record.phase, Phase::Failed { .. });
            record.phase = Phase::Destroyed { at };
            if !failed {
                record.owner.give_back_slot();
            }
        }
    }

    /// Claims every ready sandbox whose tenant `reap_reason` names, or whose lifetime has run
    /// out, for the reaper to destroy (contract C7), with the reason it is ending.
    pub fn claim_expired(
        &self,
        now: Instant,
        reap_reason: impl Fn(&str) -> Option<ExpireReason>,
    ) -> Vec<(SandboxId, Owner, ExpireReason)> {
        let mut records = self.lock();
        let mut expired = Vec::new();
        for (id, record) in records.iter_mut() {
            if !matches!(record.phase, Phase::Ready) || record.in_flight > 0 {
                continue;
            }
            let reason = reap_reason(&record.owner.tenant_id)
                .or_else(|| record.clock.expired(now).then_some(ExpireReason::Timeout));
            if let Some(reason) = reason {
                record.phase = Phase::Busy;
                expired.push((id.clone(), record.owner.clone(), reason));
            }
        }
        expired
    }

    /// Records activity on a live sandbox, restarting its idle timer. A tunnel's traffic calls
    /// this so a browser session that moves bytes is not reaped for being idle.
    pub fn touch(&self, id: &SandboxId) {
        if let Some(record) = self.lock().get_mut(id)
            && matches!(record.phase, Phase::Ready | Phase::Busy)
        {
            record.clock.touch(Instant::now());
        }
    }

    /// Forgets the records kept only so a later call can read what happened to them: a destroyed
    /// sandbox, and a launch that failed after its create had already answered.
    pub fn sweep(&self, now: Instant) {
        self.lock().retain(|_, record| match record.phase {
            Phase::Destroyed { at } | Phase::Failed { at, .. } => {
                now.saturating_duration_since(at) < TOMBSTONE_RETENTION
            }
            _ => true,
        });
    }
}
