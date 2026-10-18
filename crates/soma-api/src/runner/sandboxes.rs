use std::{
    collections::HashMap,
    sync::{Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

pub use crate::runner::owner::{LaunchFailure, Owner, Unavailable};
use crate::runner::{
    idle::{Clock, Lifetime},
    ids::SandboxId,
    starting::Starting,
};

mod records;
mod reservation;

pub use records::{Held, TOMBSTONE_RETENTION};
use records::{Phase, Record};
pub use reservation::Reservation;

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

    /// Hands a reserved sandbox over to the launch a create has answered `201` for.
    ///
    /// From here the record holds the launch rather than the request that started it, so a call
    /// that arrives before the launch ends can wait for it and read what it reported.
    pub fn start_launch(&self, id: &SandboxId, launch: Starting) {
        if let Some(record) = self.lock().get_mut(id) {
            record.phase = Phase::Starting(launch);
        }
    }

    /// Records that the launch of a reserved sandbox failed, and gives its tenant slot back.
    ///
    /// This is the accounting a failed synchronous create does, at the same moment: the slot goes
    /// back as the launch is known to have failed rather than when someone reads about it. The
    /// record itself is kept as a tombstone, so the next call reads the create's own failure.
    pub fn fail_launch(&self, id: &SandboxId, failure: LaunchFailure) {
        if let Some(record) = self.lock().get_mut(id)
            && matches!(record.phase, Phase::Creating | Phase::Starting(_))
        {
            record.phase = Phase::Failed {
                failure,
                at: Instant::now(),
            };
            record.owner.give_back_slot();
        }
    }

    /// The in-flight launch of a sandbox `tenant_id` owns, if its create answered early.
    ///
    /// `Ok(None)` is a sandbox that is not starting: a call may go on and address it, and will
    /// read whatever state it is in. Anything else is why the call cannot go on at all, including
    /// a launch that has already failed, which is answered as the synchronous create would have
    /// answered it.
    ///
    /// # Errors
    ///
    /// Returns why the sandbox cannot be addressed.
    pub fn in_flight(
        &self,
        id: &SandboxId,
        tenant_id: &str,
    ) -> Result<Option<Starting>, Unavailable> {
        let records = self.lock();
        let record = records
            .get(id)
            .filter(|record| record.owner.tenant_id == tenant_id)
            .ok_or(Unavailable::NotFound)?;
        match &record.phase {
            Phase::Starting(launch) => Ok(Some(launch.clone())),
            Phase::Failed { failure, .. } => Err(Unavailable::LaunchFailed(*failure)),
            Phase::Destroyed { at } => Err(Unavailable::Destroyed(
                record.owner.clone(),
                at.saturating_duration_since(record.owner.created),
            )),
            Phase::Creating | Phase::Ready | Phase::Busy => Ok(None),
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
            // Both are the same answer: a sandbox a create has not finished is not one this
            // caller may address. A call on a sandbox whose launch is still in flight waits for
            // it before it gets here ([`Super::in_flight`]).
            Phase::Creating | Phase::Starting(_) => Err(Unavailable::NotFound),
            Phase::Failed { failure, .. } => Err(Unavailable::LaunchFailed(failure)),
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
                    && matches!(
                        record.phase,
                        Phase::Starting(_) | Phase::Ready | Phase::Busy
                    )
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
            Phase::Creating | Phase::Starting(_) => Err(Unavailable::NotFound),
            Phase::Failed { failure, .. } => Err(Unavailable::LaunchFailed(failure)),
            Phase::Destroyed { at } => Err(Unavailable::Destroyed(
                record.owner.clone(),
                at.saturating_duration_since(record.owner.created),
            )),
        }
    }

    /// Ends a command, or a destroy that did not happen, leaving the sandbox ready.
    pub fn release(&self, id: &SandboxId) {
        if let Some(record) = self.lock().get_mut(id)
            && matches!(record.phase, Phase::Busy)
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
            Phase::Creating | Phase::Starting(_) => Err(Unavailable::NotFound),
            Phase::Failed { failure, .. } => Err(Unavailable::LaunchFailed(failure)),
            Phase::Destroyed { at } => Err(Unavailable::Destroyed(
                record.owner.clone(),
                at.saturating_duration_since(record.owner.created),
            )),
        }
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

    /// The number of sandboxes this host holds.
    ///
    /// A tombstone is not a sandbox: a destroyed one is gone, and one whose launch failed was
    /// never created. Its record is kept only so a later call can read why.
    #[must_use]
    pub fn live(&self) -> usize {
        self.lock()
            .values()
            .filter(|record| record.phase.live())
            .count()
    }

    pub(in crate::runner::sandboxes) fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<SandboxId, Record>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "sandboxes_tests.rs"]
mod tests;
