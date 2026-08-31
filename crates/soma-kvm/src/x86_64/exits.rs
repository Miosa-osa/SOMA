//! What one vCPU thread does on each side of `KVM_RUN`.
//!
//! The receipt reports the machine being ready and says nothing about where the resume spent
//! its time, and the two halves of the existing instrumentation cannot see between the vCPU
//! being armed and the guest reaching its first observable act. This ledger splits that window
//! by the only boundary that exists in userspace: the moment `KVM_RUN` is entered, the moment
//! it returns, and what it returned.
//!
//! It is deliberately blind to the exits KVM resolves in the kernel without returning here,
//! which is the point: a large gap between the first entry and the first return, with almost
//! nothing counted, says the time is being spent inside KVM rather than in this loop, and that
//! is a different problem with a different fix.
//!
//! The clock is read at most twice per return and only for the first [`SAMPLE_LIMIT`] returns,
//! so the instrument costs tens of microseconds on a launch and nothing after that.

use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use kvm_ioctls::VcpuExit;

#[cfg(test)]
mod tests;

/// Returns from `KVM_RUN` that are timed as well as counted.
///
/// A launch reaches a few hundred of these before the machine is ready, so the limit covers
/// the whole window the timeline is asked about while bounding what the instrument can cost.
pub const SAMPLE_LIMIT: u32 = 1024;

/// Unset sentinel for a recorded offset. Zero is a legitimate offset and cannot be one.
const UNSET: u64 = u64::MAX;

/// The classes of `KVM_RUN` return one sandbox can observe from userspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitReason {
    /// The guest read a port the host device models answer.
    PortIn,
    /// The guest wrote a port the host device models answer.
    PortOut,
    /// The guest touched a virtio MMIO window.
    Mmio,
    /// The guest executed `hlt`.
    Halt,
    /// The run was interrupted, by a kick or by a signal.
    Interrupted,
    /// Anything else, including a shutdown or a failed entry.
    Other,
}

impl ExitReason {
    /// Every class, in the order the counters hold them.
    pub const ALL: [Self; 6] = [
        Self::PortIn,
        Self::PortOut,
        Self::Mmio,
        Self::Halt,
        Self::Interrupted,
        Self::Other,
    ];

    const fn index(self) -> usize {
        match self {
            Self::PortIn => 0,
            Self::PortOut => 1,
            Self::Mmio => 2,
            Self::Halt => 3,
            Self::Interrupted => 4,
            Self::Other => 5,
        }
    }

    /// The class name, stable enough to key a diagnostic by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PortIn => "port_in",
            Self::PortOut => "port_out",
            Self::Mmio => "mmio",
            Self::Halt => "halt",
            Self::Interrupted => "interrupted",
            Self::Other => "other",
        }
    }

    fn of(exit: &Result<VcpuExit<'_>, kvm_ioctls::Error>) -> Self {
        match exit {
            Ok(VcpuExit::IoIn(_, _)) => Self::PortIn,
            Ok(VcpuExit::IoOut(_, _)) => Self::PortOut,
            Ok(VcpuExit::MmioRead(_, _) | VcpuExit::MmioWrite(_, _)) => Self::Mmio,
            Ok(VcpuExit::Hlt) => Self::Halt,
            Ok(VcpuExit::Intr) => Self::Interrupted,
            Err(error) if error.errno() == libc::EINTR => Self::Interrupted,
            Ok(_) | Err(_) => Self::Other,
        }
    }
}

/// What one vCPU's run loop counted and timed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExitCounts {
    per_reason: [u32; ExitReason::ALL.len()],
    /// Returns that were timed as well as counted.
    pub sampled: u32,
    /// Time inside `KVM_RUN` across the sampled returns.
    pub inside_ns: u64,
    /// Time in the userspace loop between the sampled returns.
    pub outside_ns: u64,
    /// How long the very first `KVM_RUN` took to return.
    pub first_run_ns: u64,
}

impl ExitCounts {
    /// How many returns of `reason` the loop saw.
    #[must_use]
    pub const fn of(&self, reason: ExitReason) -> u32 {
        self.per_reason[reason.index()]
    }

    /// Every return the loop saw, of every class.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.per_reason.iter().copied().fold(0, u32::saturating_add)
    }
}

/// The counters one vCPU thread writes and its owner reads after the thread has stopped.
#[derive(Debug)]
pub(crate) struct ExitLedger {
    started: Instant,
    entered_ns: AtomicU64,
    returned_ns: AtomicU64,
    per_reason: [AtomicU32; ExitReason::ALL.len()],
    sampled: AtomicU32,
    inside_ns: AtomicU64,
    outside_ns: AtomicU64,
}

impl ExitLedger {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            entered_ns: AtomicU64::new(UNSET),
            returned_ns: AtomicU64::new(UNSET),
            per_reason: std::array::from_fn(|_| AtomicU32::new(0)),
            sampled: AtomicU32::new(0),
            inside_ns: AtomicU64::new(0),
            outside_ns: AtomicU64::new(0),
        }
    }

    /// A recorder for one run loop. Only the loop's own thread may hold it.
    pub(crate) const fn sampler(&self) -> Sampler<'_> {
        Sampler {
            ledger: self,
            entered_at: None,
            left_at: None,
        }
    }

    /// The moment the loop was about to make its first `KVM_RUN` call.
    pub(crate) fn first_entry(&self) -> Option<Instant> {
        self.instant(&self.entered_ns)
    }

    /// The moment the first `KVM_RUN` call returned.
    pub(crate) fn first_return(&self) -> Option<Instant> {
        self.instant(&self.returned_ns)
    }

    fn instant(&self, cell: &AtomicU64) -> Option<Instant> {
        match cell.load(Ordering::Acquire) {
            UNSET => None,
            offset => Some(self.started + Duration::from_nanos(offset)),
        }
    }

    /// Everything counted so far. Read after the vCPU thread has stopped.
    pub(crate) fn counts(&self) -> ExitCounts {
        let entered = self.entered_ns.load(Ordering::Acquire);
        let returned = self.returned_ns.load(Ordering::Acquire);
        ExitCounts {
            per_reason: std::array::from_fn(|index| self.per_reason[index].load(Ordering::Acquire)),
            sampled: self.sampled.load(Ordering::Acquire),
            inside_ns: self.inside_ns.load(Ordering::Acquire),
            outside_ns: self.outside_ns.load(Ordering::Acquire),
            first_run_ns: match (entered, returned) {
                (UNSET, _) | (_, UNSET) => 0,
                (entered, returned) => returned.saturating_sub(entered),
            },
        }
    }

    fn offset(&self, at: Instant) -> u64 {
        u64::try_from(at.duration_since(self.started).as_nanos()).unwrap_or(u64::MAX - 1)
    }

    fn set_once(&self, cell: &AtomicU64, at: Instant) {
        let _ignored =
            cell.compare_exchange(UNSET, self.offset(at), Ordering::AcqRel, Ordering::Relaxed);
    }
}

/// One run loop's recorder, held on the vCPU thread for the life of the loop.
pub(crate) struct Sampler<'a> {
    ledger: &'a ExitLedger,
    entered_at: Option<Instant>,
    left_at: Option<Instant>,
}

impl Sampler<'_> {
    /// Records that the loop is about to call `KVM_RUN`.
    pub(crate) fn entering(&mut self) {
        if self.ledger.sampled.load(Ordering::Acquire) >= SAMPLE_LIMIT {
            self.entered_at = None;
            return;
        }
        let now = Instant::now();
        self.ledger.set_once(&self.ledger.entered_ns, now);
        if let Some(left) = self.left_at {
            add(&self.ledger.outside_ns, now.saturating_duration_since(left));
        }
        self.entered_at = Some(now);
    }

    /// Records what `KVM_RUN` returned, and how long it took when the return is sampled.
    pub(crate) fn returned(&mut self, exit: &Result<VcpuExit<'_>, kvm_ioctls::Error>) {
        let reason = ExitReason::of(exit);
        self.ledger.per_reason[reason.index()].fetch_add(1, Ordering::AcqRel);
        let Some(entered) = self.entered_at.take() else {
            return;
        };
        let now = Instant::now();
        self.ledger.set_once(&self.ledger.returned_ns, now);
        add(
            &self.ledger.inside_ns,
            now.saturating_duration_since(entered),
        );
        self.ledger.sampled.fetch_add(1, Ordering::AcqRel);
        self.left_at = Some(now);
    }
}

fn add(cell: &AtomicU64, elapsed: Duration) {
    let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
    cell.fetch_add(nanos, Ordering::AcqRel);
}
