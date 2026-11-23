//! Extend, recover and count the sandboxes this runner holds.

use super::{
    Clock, Duration, HashMap, Instant, Lifetime, Ordering, Owner, Phase, Record, SandboxId,
    Sandboxes, Unavailable,
};

impl Sandboxes {
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
