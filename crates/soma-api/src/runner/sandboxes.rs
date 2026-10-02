use std::{
    collections::HashMap,
    sync::{Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

pub use crate::runner::owner::{Owner, Unavailable};
use crate::runner::{
    idle::{Clock, Lifetime},
    ids::SandboxId,
    journal::ExpireReason,
};

/// One call holding a sandbox (see [`Sandboxes::hold`]); dropping it lets the sweep back in.
pub struct Held<'a> {
    sandboxes: &'a Sandboxes,
    id: SandboxId,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Reserved by a create that has not finished.
    Creating,
    Ready,
    /// One command or destroy is in flight.
    Busy,
    Destroyed {
        at: Instant,
    },
}

#[derive(Clone, Debug)]
struct Record {
    owner: Owner,
    clock: Clock,
    phase: Phase,
    /// Calls that hold the sandbox without its lifecycle slot (inspect, files, terminal).
    /// They overlap freely with each other and with a command; the sweep waits for all.
    in_flight: u32,
}

/// The runner's own record of which tenant owns each sandbox on this host.
///
/// The facade does not scope sandboxes by tenant, so this is what stops one tenant commanding
/// another's sandbox. It also serializes lifecycle calls per sandbox, as the fast lane's route
/// table did: one command at a time, and no destroy under a running command.
#[derive(Debug, Default)]
pub struct Sandboxes {
    records: Mutex<HashMap<SandboxId, Record>>,
}

impl Sandboxes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new sandbox for `owner`, whose tenant slot the caller has already taken.
    pub fn reserve(&self, id: SandboxId, owner: Owner, lifetime: Lifetime) {
        let clock = Clock::start(lifetime, owner.created);
        self.lock().insert(
            id,
            Record {
                owner,
                clock,
                phase: Phase::Creating,
                in_flight: 0,
            },
        );
    }

    /// Marks a reserved sandbox as created and ready for commands.
    pub fn confirm(&self, id: &SandboxId) {
        if let Some(record) = self.lock().get_mut(id) {
            record.phase = Phase::Ready;
        }
    }

    /// Forgets a reservation whose create failed, giving its tenant slot back.
    pub fn abandon(&self, id: &SandboxId) {
        if let Some(record) = self.lock().remove(id) {
            record.owner.give_back_slot();
        }
    }

    /// Holds a sandbox `tenant_id` owns for one inspect, file or terminal call until the
    /// guard drops: a counter, not the exclusive Busy phase, so such calls still overlap with
    /// each other and with a command. It keeps the sweep away; its end restarts the timer.
    ///
    /// # Errors
    ///
    /// Returns why the sandbox cannot be addressed.
    pub fn hold(&self, id: &SandboxId, tenant_id: &str) -> Result<Held<'_>, Unavailable> {
        let mut records = self.lock();
        let record = records
            .get_mut(id)
            .filter(|record| record.owner.tenant_id == tenant_id)
            .ok_or(Unavailable::NotFound)?;
        match record.phase {
            Phase::Ready | Phase::Busy => {
                record.clock.touch(Instant::now());
                record.in_flight += 1;
                Ok(Held {
                    sandboxes: self,
                    id: id.clone(),
                    owner: record.owner.clone(),
                })
            }
            Phase::Creating => Err(Unavailable::NotFound),
            Phase::Destroyed { at } => Err(Unavailable::Destroyed(
                record.owner.clone(),
                at.saturating_duration_since(record.owner.created),
            )),
        }
    }

    /// The ids of the live sandboxes `tenant_id` owns, for a tenant-scoped listing.
    #[must_use]
    pub fn owned_by(&self, tenant_id: &str) -> Vec<SandboxId> {
        self.lock()
            .iter()
            .filter(|(_, record)| {
                record.owner.tenant_id == tenant_id
                    && matches!(record.phase, Phase::Ready | Phase::Busy)
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Starts one command on a sandbox `tenant_id` owns.
    ///
    /// # Errors
    ///
    /// Returns why the command cannot start.
    pub fn begin_command(&self, id: &SandboxId, tenant_id: &str) -> Result<Owner, Unavailable> {
        self.begin(id, tenant_id)
    }

    /// Starts the destroy of a sandbox `tenant_id` owns.
    ///
    /// # Errors
    ///
    /// Returns why the destroy cannot start; [`Unavailable::Destroyed`] means it already
    /// happened and the caller answers as the first destroy did.
    pub fn begin_destroy(&self, id: &SandboxId, tenant_id: &str) -> Result<Owner, Unavailable> {
        self.begin(id, tenant_id)
    }

    fn begin(&self, id: &SandboxId, tenant_id: &str) -> Result<Owner, Unavailable> {
        let mut records = self.lock();
        let record = records
            .get_mut(id)
            .filter(|record| record.owner.tenant_id == tenant_id)
            .ok_or(Unavailable::NotFound)?;
        match record.phase {
            Phase::Ready => {
                record.phase = Phase::Busy;
                record.clock.touch(Instant::now());
                Ok(record.owner.clone())
            }
            Phase::Busy => Err(Unavailable::Busy),
            Phase::Creating => Err(Unavailable::NotFound),
            Phase::Destroyed { at } => Err(Unavailable::Destroyed(
                record.owner.clone(),
                at.saturating_duration_since(record.owner.created),
            )),
        }
    }

    /// Ends a command, or a destroy that did not happen, leaving the sandbox ready.
    pub fn release(&self, id: &SandboxId) {
        if let Some(record) = self.lock().get_mut(id)
            && record.phase == Phase::Busy
        {
            record.phase = Phase::Ready;
            // A command's end is activity too: a long command must not leave its sandbox
            // already idle when it returns.
            record.clock.touch(Instant::now());
        }
    }

    /// Replaces the idle timeout of a sandbox `tenant_id` owns and resets its timer.
    ///
    /// Returns when it now ends, or `None` if nothing will end it.
    ///
    /// # Errors
    ///
    /// Returns why the sandbox cannot be addressed.
    pub fn extend(
        &self,
        id: &SandboxId,
        tenant_id: &str,
        idle: Option<Duration>,
    ) -> Result<Option<Instant>, Unavailable> {
        let mut records = self.lock();
        let record = records
            .get_mut(id)
            .filter(|record| record.owner.tenant_id == tenant_id)
            .ok_or(Unavailable::NotFound)?;
        match record.phase {
            Phase::Ready | Phase::Busy => {
                record.clock.extend(idle, Instant::now());
                Ok(record.clock.deadline())
            }
            Phase::Creating => Err(Unavailable::NotFound),
            Phase::Destroyed { at } => Err(Unavailable::Destroyed(
                record.owner.clone(),
                at.saturating_duration_since(record.owner.created),
            )),
        }
    }

    /// Records that a sandbox is gone, giving its tenant slot back exactly once.
    pub fn destroyed(&self, id: &SandboxId, at: Instant) {
        if let Some(record) = self.lock().get_mut(id)
            && !matches!(record.phase, Phase::Destroyed { .. })
        {
            record.phase = Phase::Destroyed { at };
            record.owner.give_back_slot();
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
            if record.phase != Phase::Ready || record.in_flight > 0 {
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

    /// Forgets destroyed sandboxes older than the retention.
    pub fn sweep(&self, now: Instant) {
        self.lock().retain(|_, record| match record.phase {
            Phase::Destroyed { at } => now.saturating_duration_since(at) < TOMBSTONE_RETENTION,
            _ => true,
        });
    }

    /// Re-adopts a sandbox the state store still holds after a restart, counting it against
    /// its tenant.
    pub fn recover(&self, id: SandboxId, owner: Owner, lifetime: Lifetime) {
        let clock = Clock::start(lifetime, owner.created);
        if let std::collections::hash_map::Entry::Vacant(vacant) = self.lock().entry(id) {
            owner.slot.fetch_add(1, Ordering::AcqRel);
            vacant.insert(Record {
                owner,
                clock,
                phase: Phase::Ready,
                in_flight: 0,
            });
        }
    }

    /// The number of sandboxes this host holds that are not destroyed.
    #[must_use]
    pub fn live(&self) -> usize {
        self.lock()
            .values()
            .filter(|record| !matches!(record.phase, Phase::Destroyed { .. }))
            .count()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SandboxId, Record>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "sandboxes_tests.rs"]
mod tests;
