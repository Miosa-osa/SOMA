//! Typestated worker handles: only legal transitions compile, and every transition is the
//! compare-and-swap on the shared [`Slot`] so stale handles fail at run time too.

use std::{marker::PhantomData, sync::Arc};

use super::{Packed, Phase, Slot, StateRace};
use crate::{LeaseGeneration, WorkerId};

mod sealed {
    pub trait Sealed {}
}

/// A compile-time phase marker.
pub trait Phased: sealed::Sealed + Send + Sync + 'static {
    /// The phase the marker represents.
    const PHASE: Phase;
}

macro_rules! phase_marker {
    ($name:ident, $phase:expr) => {
        #[doc = concat!("Marker for the `", stringify!($name), "` phase.")]
        #[derive(Clone, Copy, Debug)]
        pub struct $name;

        impl sealed::Sealed for $name {}

        impl Phased for $name {
            const PHASE: Phase = $phase;
        }
    };
}

phase_marker!(Constructing, Phase::Constructing);
phase_marker!(Sterile, Phase::Sterile);
phase_marker!(Claiming, Phase::Claiming);
phase_marker!(Assigned, Phase::Assigned);
phase_marker!(Running, Phase::Running);
phase_marker!(Destroying, Phase::Destroying);
phase_marker!(Dead, Phase::Dead);

/// One handle on a worker in exactly one phase and lease generation.
#[derive(Debug)]
pub struct Worker<S: Phased> {
    slot: Arc<Slot>,
    generation: LeaseGeneration,
    _phase: PhantomData<S>,
}

impl<S: Phased> Worker<S> {
    /// Returns the worker identity.
    #[must_use]
    pub fn id(&self) -> WorkerId {
        self.slot.id()
    }

    /// Returns the lease generation this handle acts on.
    #[must_use]
    pub const fn generation(&self) -> LeaseGeneration {
        self.generation
    }

    /// Returns the shared slot.
    #[must_use]
    pub const fn slot(&self) -> &Arc<Slot> {
        &self.slot
    }

    /// Returns the phase this handle is typed for.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        S::PHASE
    }

    /// Attaches a handle to a slot that is already in this phase.
    #[must_use]
    pub fn attach(slot: Arc<Slot>) -> Option<Self> {
        let observed = slot.observe();
        (observed.phase == S::PHASE).then_some(Self {
            slot,
            generation: observed.generation,
            _phase: PhantomData,
        })
    }

    fn advance<N: Phased>(self, generation: LeaseGeneration) -> Result<Worker<N>, StateRace> {
        let from = Packed {
            phase: S::PHASE,
            generation: self.generation,
        };
        let to = Packed {
            phase: N::PHASE,
            generation,
        };
        self.slot.word().transition(from, to)?;
        Ok(Worker {
            slot: self.slot,
            generation,
            _phase: PhantomData,
        })
    }

    fn keep<N: Phased>(self) -> Result<Worker<N>, StateRace> {
        let generation = self.generation;
        self.advance(generation)
    }
}

impl Worker<Constructing> {
    /// Takes the constructing handle of a fresh slot.
    #[must_use]
    pub fn open(slot: Arc<Slot>) -> Self {
        Self {
            generation: slot.observe().generation,
            slot,
            _phase: PhantomData,
        }
    }

    /// Construction finished with only invariant state.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn sterilize(self) -> Result<Worker<Sterile>, StateRace> {
        self.keep()
    }

    /// Construction failed; nothing was ever claimable.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn abort(self) -> Result<Worker<Dead>, StateRace> {
        self.keep()
    }
}

impl Worker<Sterile> {
    /// Claims the worker, bumping the lease generation.
    ///
    /// # Errors
    ///
    /// Returns the race when another claimer or an eviction moved the slot first.
    pub fn claim(self) -> Result<Worker<Claiming>, StateRace> {
        let next = self
            .generation
            .next()
            .map_err(|_| StateRace::GenerationRule)?;
        self.advance(next)
    }

    /// Evicts a sterile worker, for example when its Generation is retired.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn evict(self) -> Result<Worker<Destroying>, StateRace> {
        self.keep()
    }
}

impl Slot {
    /// Attempts the single claim compare-and-swap; exactly one caller wins per generation.
    #[must_use]
    pub fn try_claim(self: &Arc<Self>) -> Option<Worker<Claiming>> {
        let observed = self.observe();
        if observed.phase != Phase::Sterile {
            return None;
        }
        Worker::<Sterile> {
            slot: Arc::clone(self),
            generation: observed.generation,
            _phase: PhantomData,
        }
        .claim()
        .ok()
    }

    /// Attempts to evict a sterile worker.
    #[must_use]
    pub fn try_evict(self: &Arc<Self>) -> Option<Worker<Destroying>> {
        let observed = self.observe();
        if observed.phase != Phase::Sterile {
            return None;
        }
        Worker::<Sterile> {
            slot: Arc::clone(self),
            generation: observed.generation,
            _phase: PhantomData,
        }
        .evict()
        .ok()
    }
}

impl Worker<Claiming> {
    /// Every authority step was acknowledged.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn assign(self) -> Result<Worker<Assigned>, StateRace> {
        self.keep()
    }

    /// The transfer was ambiguous or failed; the worker never returns to the pool.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn destroy(self) -> Result<Worker<Destroying>, StateRace> {
        self.keep()
    }
}

impl Worker<Assigned> {
    /// The Instance started executing.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn run(self) -> Result<Worker<Running>, StateRace> {
        self.keep()
    }

    /// Teardown before the Instance ran.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn destroy(self) -> Result<Worker<Destroying>, StateRace> {
        self.keep()
    }
}

impl Worker<Running> {
    /// The Instance stopped; the single-use worker is torn down.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn destroy(self) -> Result<Worker<Destroying>, StateRace> {
        self.keep()
    }
}

impl Worker<Destroying> {
    /// Every resource is released.
    ///
    /// # Errors
    ///
    /// Returns the race when another actor moved the slot.
    pub fn finish(self) -> Result<Worker<Dead>, StateRace> {
        self.keep()
    }
}
