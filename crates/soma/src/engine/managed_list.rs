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

use crate::{Backend, BackendKind, InstanceId, MachineName, SandboxLiveness, StateStore};

use super::{
    Engine, ManagedFailure,
    machine_state::{DurablePhase, VersionedMachine},
    managed_state::store_failure,
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Lists every sandbox this engine's durable store holds that has not reached a terminal
    /// phase, with what the backend can still reach for each.
    ///
    /// # Errors
    ///
    /// Returns a typed durable-store failure. One record that cannot be decoded fails the whole
    /// listing rather than being skipped: a listing that silently dropped a record would report
    /// a sandbox as absent on the strength of a corrupt document.
    pub fn list_machines(&mut self) -> Result<Vec<SandboxEntry>, ManagedFailure> {
        let mut identities = self.state.list().map_err(store_failure)?;
        // The store is a directory of records with no inherent order, so the listing sorts by
        // identity. An unordered answer would differ between two calls that found the same set.
        identities.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let mut entries = Vec::new();
        for instance_id in identities {
            let Some(stored) = self.load_machine(&instance_id)? else {
                // The record was released between the enumeration and this read. It is gone, and
                // reporting it would name a sandbox that no longer exists.
                continue;
            };
            if let Some(entry) = self.entry(instance_id, &stored) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// Builds one entry, or nothing for a record that names no live sandbox.
    fn entry(
        &mut self,
        instance_id: InstanceId,
        stored: &VersionedMachine,
    ) -> Option<SandboxEntry> {
        let (phase, backend, name) = match &stored.machine.phase {
            DurablePhase::Launching { intent } => (
                SandboxPhase::Launching,
                intent.backend,
                intent.machine_name.clone(),
            ),
            DurablePhase::Active { active } => (
                SandboxPhase::Active,
                active.backend(),
                active.machine_name().cloned(),
            ),
            DurablePhase::Executing { active, .. } => (
                SandboxPhase::Executing,
                active.backend(),
                active.machine_name().cloned(),
            ),
            DurablePhase::Terminating { active, .. } => (
                SandboxPhase::Terminating,
                active.backend(),
                active.machine_name().cloned(),
            ),
            // A released Instance is not a sandbox any more.
            DurablePhase::Terminal { .. } => return None,
        };
        // A record written by a different backend belongs to a machine this engine cannot reach,
        // so it is reported without a liveness claim rather than probed against the wrong one.
        let liveness = if backend == self.backend.kind() {
            self.backend.liveness(&instance_id)
        } else {
            SandboxLiveness::Unknown
        };
        Some(SandboxEntry {
            instance_id,
            phase,
            backend,
            name,
            liveness,
        })
    }
}

/// What the durable record says one sandbox's last completed transition left it in.
///
/// This is the store's phase rather than the guest's state. A machine reported Active here has a
/// record saying its launch completed and nothing has released it; whether its guest is running
/// is what the liveness beside it is for, and what an inspection by exact identity answers in
/// full.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxPhase {
    /// A launch was admitted and has not completed.
    Launching,
    /// The launch completed and nothing has released it.
    Active,
    /// A command is in flight against it.
    Executing,
    /// A release is in flight against it.
    Terminating,
}

impl SandboxPhase {
    /// The stable name a surface reports this phase by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Launching => "launching",
            Self::Active => "active",
            Self::Executing => "executing",
            Self::Terminating => "terminating",
        }
    }
}

/// One sandbox in a listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxEntry {
    instance_id: InstanceId,
    phase: SandboxPhase,
    backend: BackendKind,
    name: Option<MachineName>,
    liveness: SandboxLiveness,
}

impl SandboxEntry {
    /// Builds one entry.
    ///
    /// Public because the trait a service implements to serve a listing returns these, so a
    /// facade that is not this engine still has to be able to produce one.
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        phase: SandboxPhase,
        backend: BackendKind,
        name: Option<MachineName>,
        liveness: SandboxLiveness,
    ) -> Self {
        Self {
            instance_id,
            phase,
            backend,
            name,
            liveness,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn phase(&self) -> SandboxPhase {
        self.phase
    }

    #[must_use]
    pub const fn backend(&self) -> BackendKind {
        self.backend
    }

    /// The optional metadata name the launch carried, which never replaces the identity.
    #[must_use]
    pub const fn name(&self) -> Option<&MachineName> {
        self.name.as_ref()
    }

    /// What the backend could reach for this Instance at the moment of the listing.
    #[must_use]
    pub const fn liveness(&self) -> SandboxLiveness {
        self.liveness
    }
}
