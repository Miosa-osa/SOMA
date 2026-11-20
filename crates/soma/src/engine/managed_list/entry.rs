//! One sandbox in a listing, and the phase its durable record is in.

use crate::{BackendKind, GenerationId, InstanceId, MachineName, SandboxLiveness};

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
    generation_id: Option<GenerationId>,
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
            generation_id: None,
        }
    }

    /// The same entry, naming the Generation its durable record says it was launched from.
    #[must_use]
    pub fn with_generation(mut self, generation_id: Option<GenerationId>) -> Self {
        self.generation_id = generation_id;
        self
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

    /// The Generation the durable record says this Instance was launched from.
    ///
    /// Absent when the record names no Generation: a backend that resolves no Generation, or a
    /// launch whose workload was never resolved. A caller deciding whether a Generation is still
    /// in use must treat an absent value as "could be any", never as "none".
    #[must_use]
    pub const fn generation_id(&self) -> Option<&GenerationId> {
        self.generation_id.as_ref()
    }
}
