//! Enumerating the sandboxes this engine's durable state knows about.
//!
//! Two different facts have to be reported here and they must not be collapsed into one. The
//! durable store says an Instance was admitted and records the phase its last completed
//! transition left it in. It cannot say whether the process holding that Instance's machine is
//! still running, because nothing writes to the record when a process dies: a host killed with
//! `SIGKILL`, or a machine lost with the whole host, leaves an Active record behind and no
//! machine anywhere.
//!
//! So a listing carries both, side by side and separately named. The phase is what the store
//! says. The liveness is what the backend could still reach, asked for each record at the moment
//! of the listing. A caller looking for a sandbox it can use reads both; a caller shown one
//! number would have been shown a guess.
//!
//! A released Instance is not in the listing at all. Its terminal record is durable evidence that
//! it existed and was destroyed, and it is still readable by exact identity, but a list of
//! sandboxes that included destroyed ones would answer a question nobody asked.

use crate::{
    Backend, BackendKind, InstanceId, MachineName, SandboxLiveness, StateStore,
    StateStoreFailureKind, WorkloadIdentity,
};

use super::{
    Engine, ManagedFailure,
    machine_state::{ActiveMachine, DurablePhase, VersionedMachine},
    managed_state::store_failure,
};

mod entry;

pub use entry::{SandboxEntry, SandboxPhase};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Lists every sandbox this engine's durable store holds that has not reached a terminal
    /// phase, with what the backend can still reach for each.
    ///
    /// # Errors
    ///
    /// Returns a typed durable-store failure. One record that cannot be decoded fails the whole
    /// listing rather than being skipped: a listing that silently dropped a record would report
    /// a sandbox as absent on the strength of a corrupt document. A caller that would rather be
    /// told about such a record than lose the rest of the listing uses
    /// [`Self::list_machines_reporting_unreadable`].
    pub fn list_machines(&mut self) -> Result<Vec<SandboxEntry>, ManagedFailure> {
        self.list_all(false).map(|listing| listing.entries)
    }

    /// Lists the sandboxes like [`Self::list_machines`], but a record that cannot be decoded is
    /// named in the answer instead of failing it.
    ///
    /// The unreadable records are reported, never dropped: the listing says which identities it
    /// could not read and why, so a caller cannot take their absence for the absence of the
    /// sandboxes. Only a record that was read and found invalid, corrupt, or of an unsupported
    /// version is treated this way; a store that cannot be read at all still fails the call.
    ///
    /// # Errors
    ///
    /// Returns a typed durable-store failure when the store cannot be enumerated or read.
    pub fn list_machines_reporting_unreadable(&mut self) -> Result<MachineListing, ManagedFailure> {
        self.list_all(true)
    }

    fn list_all(&mut self, report_unreadable: bool) -> Result<MachineListing, ManagedFailure> {
        let mut identities = self.state.list().map_err(store_failure)?;
        // The store is a directory of records with no inherent order, so the listing sorts by
        // identity. An unordered answer would differ between two calls that found the same set.
        identities.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let mut listing = MachineListing::default();
        for instance_id in identities {
            let stored = match self.load_machine(&instance_id) {
                Ok(Some(stored)) => stored,
                // The record was released between the enumeration and this read. It is gone, and
                // reporting it would name a sandbox that no longer exists.
                Ok(None) => continue,
                Err(ManagedFailure::StateStore(kind))
                    if report_unreadable && is_unreadable_record(kind) =>
                {
                    listing
                        .unreadable
                        .push(UnreadableRecord { instance_id, kind });
                    continue;
                }
                Err(failure) => return Err(failure),
            };
            if let Some(entry) = self.entry(instance_id, &stored) {
                listing.entries.push(entry);
            }
        }
        Ok(listing)
    }

    /// Builds one entry, or nothing for a record that names no live sandbox.
    fn entry(
        &mut self,
        instance_id: InstanceId,
        stored: &VersionedMachine,
    ) -> Option<SandboxEntry> {
        let (phase, launch) = match &stored.machine.phase {
            DurablePhase::Launching { intent } => (
                SandboxPhase::Launching,
                (
                    intent.backend,
                    intent.machine_name.clone(),
                    Some(&intent.workload),
                ),
            ),
            DurablePhase::Active { active } => (SandboxPhase::Active, of_active(active)),
            DurablePhase::Executing { active, .. } => (SandboxPhase::Executing, of_active(active)),
            DurablePhase::Terminating { active, .. } => {
                (SandboxPhase::Terminating, of_active(active))
            }
            // A released Instance is not a sandbox any more.
            DurablePhase::Terminal { .. } => return None,
        };
        let (backend, name, workload) = launch;
        // A record written by a different backend belongs to a machine this engine cannot reach,
        // so it is reported without a liveness claim rather than probed against the wrong one.
        let liveness = if backend == self.backend.kind() {
            self.backend.liveness(&instance_id)
        } else {
            SandboxLiveness::Unknown
        };
        let generation = workload.and_then(WorkloadIdentity::generation_id).cloned();
        Some(
            SandboxEntry::new(instance_id, phase, backend, name, liveness)
                .with_generation(generation),
        )
    }
}

/// What an admitted launch recorded: its backend, its name, and the workload it resolved.
fn of_active(
    active: &ActiveMachine,
) -> (BackendKind, Option<MachineName>, Option<&WorkloadIdentity>) {
    (
        active.backend(),
        active.machine_name().cloned(),
        active.workload(),
    )
}

/// A record the store holds that could be read from disk but not decoded as a machine.
const fn is_unreadable_record(kind: StateStoreFailureKind) -> bool {
    matches!(
        kind,
        StateStoreFailureKind::InvalidRecord
            | StateStoreFailureKind::Corrupt
            | StateStoreFailureKind::UnsupportedVersion
    )
}

/// What a listing found, with the records it could not read named beside the ones it could.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MachineListing {
    entries: Vec<SandboxEntry>,
    unreadable: Vec<UnreadableRecord>,
}

impl MachineListing {
    /// Builds a listing in which every record was readable.
    #[must_use]
    pub const fn new(entries: Vec<SandboxEntry>) -> Self {
        Self {
            entries,
            unreadable: Vec::new(),
        }
    }

    /// Builds a listing that also names the records it could not read.
    #[must_use]
    pub const fn with_unreadable(
        entries: Vec<SandboxEntry>,
        unreadable: Vec<UnreadableRecord>,
    ) -> Self {
        Self {
            entries,
            unreadable,
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[SandboxEntry] {
        &self.entries
    }

    #[must_use]
    pub fn unreadable(&self) -> &[UnreadableRecord] {
        &self.unreadable
    }

    /// Splits the listing into the readable entries and the unreadable records.
    #[must_use]
    pub fn into_parts(self) -> (Vec<SandboxEntry>, Vec<UnreadableRecord>) {
        (self.entries, self.unreadable)
    }
}

/// One record a listing could not decode, by identity and the store's reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadableRecord {
    instance_id: InstanceId,
    kind: StateStoreFailureKind,
}

impl UnreadableRecord {
    #[must_use]
    pub const fn new(instance_id: InstanceId, kind: StateStoreFailureKind) -> Self {
        Self { instance_id, kind }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Why the store could not decode it.
    #[must_use]
    pub const fn kind(&self) -> StateStoreFailureKind {
        self.kind
    }
}
