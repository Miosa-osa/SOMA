//! The worker state machine: phases, the legal transition table, the packed atomic state
//! word that every ownership change is a compare-and-swap over, and the ledger projection.
//!
//! `Constructing`, `Sterile`, `Claiming`, `Assigned`, and `Running` are the live phases;
//! `Destroying` and `Dead` are terminal in the sense that no phase ever leads back to
//! `Sterile`.
//! Only `Sterile` may be claimed, and the claim is the one transition that bumps the lease
//! generation.

mod typestate;

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub use typestate::{
    Assigned, Claiming, Constructing, Dead, Destroying, Phased, Running, Sterile, Worker,
};

use crate::{
    InstanceId, LeaseGeneration, OperationId, PoolKeyDigest, RequestFingerprint, ResourceRefs,
    TransferStep, WorkerId, WorkerIdentity,
};

/// One phase of a worker.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Phase {
    /// The process is being built; it holds no tenant byte.
    Constructing = 1,
    /// Invariant state only; the sole claimable phase.
    Sterile = 2,
    /// One claim won; authority is being transferred exactly once.
    Claiming = 3,
    /// Fresh authority was transferred; the worker belongs to one Instance.
    Assigned = 4,
    /// The Instance is executing.
    Running = 5,
    /// Teardown in progress; never returns to the pool.
    Destroying = 6,
    /// Every resource is released and the record is closed.
    Dead = 7,
}

impl Phase {
    /// Every phase in order.
    pub const ALL: [Self; 7] = [
        Self::Constructing,
        Self::Sterile,
        Self::Claiming,
        Self::Assigned,
        Self::Running,
        Self::Destroying,
        Self::Dead,
    ];

    /// Returns the stable encoding.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Decodes one phase.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Constructing),
            2 => Some(Self::Sterile),
            3 => Some(Self::Claiming),
            4 => Some(Self::Assigned),
            5 => Some(Self::Running),
            6 => Some(Self::Destroying),
            7 => Some(Self::Dead),
            _ => None,
        }
    }

    /// Returns whether the phase can never lead back to the pool.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Destroying | Self::Dead)
    }

    /// Returns whether the ledger considers the phase open after a restart.
    #[must_use]
    pub const fn is_nonterminal(self) -> bool {
        !matches!(self, Self::Dead)
    }

    /// The legal transition table.
    #[must_use]
    pub const fn may_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Constructing, Self::Sterile | Self::Dead)
                | (Self::Sterile, Self::Claiming | Self::Destroying)
                | (Self::Claiming, Self::Assigned | Self::Destroying)
                | (Self::Assigned, Self::Running | Self::Destroying)
                | (Self::Running, Self::Destroying)
                | (Self::Destroying, Self::Dead)
        )
    }
}

/// One observed phase and lease generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Packed {
    /// The phase.
    pub phase: Phase,
    /// The lease generation.
    pub generation: LeaseGeneration,
}

/// Why a transition did not happen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateRace {
    /// The table forbids the transition.
    Illegal {
        /// Origin phase.
        from: Phase,
        /// Requested phase.
        to: Phase,
    },
    /// The generation rule was violated: a claim bumps by one, everything else keeps it.
    GenerationRule,
    /// Another actor moved the worker first.
    Lost {
        /// What the caller expected.
        expected: Packed,
        /// What the word held.
        observed: Packed,
    },
}

/// The packed atomic state word: phase in the top byte, lease generation below.
#[derive(Debug)]
pub struct StateWord(AtomicU64);

const PHASE_SHIFT: u32 = 56;
const GENERATION_MASK: u64 = (1 << PHASE_SHIFT) - 1;

impl StateWord {
    /// Packs one initial state.
    #[must_use]
    pub fn new(phase: Phase, generation: LeaseGeneration) -> Self {
        Self(AtomicU64::new(pack(Packed { phase, generation })))
    }

    /// Reads the current state.
    #[must_use]
    pub fn load(&self) -> Packed {
        unpack(self.0.load(Ordering::Acquire))
    }

    /// Performs exactly one compare-and-swap from `from` to `to`.
    ///
    /// # Errors
    ///
    /// Returns [`StateRace`] when the table forbids the move, the generation rule is broken,
    /// or another actor moved the word first; the word is unchanged on every error.
    pub fn transition(&self, from: Packed, to: Packed) -> Result<(), StateRace> {
        if !from.phase.may_transition_to(to.phase) {
            return Err(StateRace::Illegal {
                from: from.phase,
                to: to.phase,
            });
        }
        let claim = from.phase == Phase::Sterile && to.phase == Phase::Claiming;
        let expected_generation = if claim {
            from.generation
                .next()
                .map_err(|_| StateRace::GenerationRule)?
        } else {
            from.generation
        };
        if to.generation != expected_generation {
            return Err(StateRace::GenerationRule);
        }
        self.0
            .compare_exchange(pack(from), pack(to), Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|observed| StateRace::Lost {
                expected: from,
                observed: unpack(observed),
            })
    }
}

fn pack(state: Packed) -> u64 {
    (u64::from(state.phase.code()) << PHASE_SHIFT) | (state.generation.get() & GENERATION_MASK)
}

fn unpack(word: u64) -> Packed {
    let code = u8::try_from(word >> PHASE_SHIFT).unwrap_or(0);
    Packed {
        phase: Phase::from_code(code).unwrap_or(Phase::Dead),
        generation: LeaseGeneration::new(word & GENERATION_MASK).unwrap_or(LeaseGeneration::MAX),
    }
}

/// One worker's shared state cell.
#[derive(Debug)]
pub struct Slot {
    id: WorkerId,
    key: PoolKeyDigest,
    word: StateWord,
}

impl Slot {
    /// Creates a slot in `Constructing` at the first generation.
    #[must_use]
    pub fn new(id: WorkerId, key: PoolKeyDigest) -> Arc<Self> {
        Arc::new(Self {
            id,
            key,
            word: StateWord::new(Phase::Constructing, LeaseGeneration::FIRST),
        })
    }

    /// Restores a slot in a recorded phase after a restart.
    #[must_use]
    pub fn restore(
        id: WorkerId,
        key: PoolKeyDigest,
        phase: Phase,
        generation: LeaseGeneration,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            key,
            word: StateWord::new(phase, generation),
        })
    }

    /// Returns the worker identity.
    #[must_use]
    pub const fn id(&self) -> WorkerId {
        self.id
    }

    /// Returns the pool key digest.
    #[must_use]
    pub const fn key(&self) -> PoolKeyDigest {
        self.key
    }

    /// Reads the current phase and generation.
    #[must_use]
    pub fn observe(&self) -> Packed {
        self.word.load()
    }

    /// Returns the state word.
    #[must_use]
    pub const fn word(&self) -> &StateWord {
        &self.word
    }
}

/// The ledger's projection of one worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerLedgerEntry {
    /// The worker.
    pub worker: WorkerId,
    /// The pool key digest.
    pub key: PoolKeyDigest,
    /// The last recorded phase.
    pub phase: Phase,
    /// The lease generation of the last record.
    pub lease_generation: LeaseGeneration,
    /// The claiming operation, once claimed.
    pub operation: Option<OperationId>,
    /// The assigned Instance, once assigned.
    pub instance: Option<InstanceId>,
    /// The claim fingerprint, once claimed.
    pub fingerprint: Option<RequestFingerprint>,
    /// The resources the worker holds.
    pub resources: ResourceRefs,
    /// The process identity, once constructed.
    pub identity: Option<WorkerIdentity>,
    /// The last acknowledged transfer step.
    pub last_step: Option<TransferStep>,
    /// Whether the worker was ever assigned; such a worker can never be sterile again.
    pub was_assigned: bool,
    /// Whether a restart marked the entry suspect.
    pub suspect: bool,
    /// Number of records folded into the entry.
    pub records: u32,
}

#[cfg(test)]
mod tests;
