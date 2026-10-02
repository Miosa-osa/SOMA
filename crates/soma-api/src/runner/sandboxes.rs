use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::runner::ids::SandboxId;

/// How long a destroyed sandbox is remembered, so a repeated destroy answers as the first did.
pub const TOMBSTONE_RETENTION: Duration = Duration::from_mins(10);

/// Who a sandbox belongs to, as the runner recorded it at create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub tenant_id: String,
    /// The creating key; absent for a sandbox recovered from the state store after a restart.
    pub key_id: Option<String>,
    pub project_id: Option<String>,
    pub created: Instant,
}

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
    expires: Instant,
    phase: Phase,
}

/// Why a lifecycle call on an existing sandbox cannot start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// Not here, not this tenant's, or still being created: all answer 404 alike, so a caller
    /// learns nothing about sandboxes it does not own.
    NotFound,
    Busy,
    Destroyed(Owner),
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

    /// Reserves a new id for `owner`, unless the tenant already holds `max_concurrent` live
    /// sandboxes on this host.
    ///
    /// Returns `false` when the tenant is at its cap.
    pub fn reserve(
        &self,
        id: SandboxId,
        owner: Owner,
        lifetime: Duration,
        max_concurrent: Option<u32>,
    ) -> bool {
        let mut records = self.lock();
        if let Some(cap) = max_concurrent {
            let live = records
                .values()
                .filter(|record| {
                    record.owner.tenant_id == owner.tenant_id
                        && !matches!(record.phase, Phase::Destroyed { .. })
                })
                .count();
            if live >= usize::try_from(cap).unwrap_or(usize::MAX) {
                return false;
            }
        }
        let expires = owner.created + lifetime;
        records.insert(
            id,
            Record {
                owner,
                expires,
                phase: Phase::Creating,
            },
        );
        true
    }

    /// Marks a reserved sandbox as created and ready for commands.
    pub fn confirm(&self, id: &SandboxId) {
        if let Some(record) = self.lock().get_mut(id) {
            record.phase = Phase::Ready;
        }
    }

    /// Forgets a reservation whose create failed.
    pub fn abandon(&self, id: &SandboxId) {
        self.lock().remove(id);
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
                Ok(record.owner.clone())
            }
            Phase::Busy => Err(Unavailable::Busy),
            Phase::Creating => Err(Unavailable::NotFound),
            Phase::Destroyed { .. } => Err(Unavailable::Destroyed(record.owner.clone())),
        }
    }

    /// Ends a command, or a destroy that did not happen, leaving the sandbox ready.
    pub fn release(&self, id: &SandboxId) {
        if let Some(record) = self.lock().get_mut(id)
            && record.phase == Phase::Busy
        {
            record.phase = Phase::Ready;
        }
    }

    /// Records that a sandbox is gone.
    pub fn destroyed(&self, id: &SandboxId, at: Instant) {
        if let Some(record) = self.lock().get_mut(id) {
            record.phase = Phase::Destroyed { at };
        }
    }

    /// Claims every ready sandbox whose lifetime has run out, for the reaper to destroy.
    pub fn claim_expired(&self, now: Instant) -> Vec<(SandboxId, Owner)> {
        let mut records = self.lock();
        let mut expired = Vec::new();
        for (id, record) in records.iter_mut() {
            if record.phase == Phase::Ready && record.expires <= now {
                record.phase = Phase::Busy;
                expired.push((id.clone(), record.owner.clone()));
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

    /// Re-adopts a sandbox the state store still holds after a restart.
    pub fn recover(&self, id: SandboxId, owner: Owner, lifetime: Duration) {
        let expires = owner.created + lifetime;
        self.lock().entry(id).or_insert(Record {
            owner,
            expires,
            phase: Phase::Ready,
        });
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
