//! Whether anything is still serving one Instance right now.
//!
//! A durable record says an Instance was admitted and has not been released. It cannot say
//! whether the process holding that Instance's machine is still running, because nothing writes
//! to the record when a process dies. Those are two different questions and the answer to the
//! second one is here.
//!
//! This is a probe, not a lifecycle operation. It performs no work inside the guest, changes
//! nothing, and mints no evidence. It exists so that enumerating sandboxes can report what the
//! durable record says *and* what the backend can still reach, instead of reporting one as
//! though it were the other.

/// What a backend can say about an Instance it is asked to reach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxLiveness {
    /// Something is serving this Instance and answered the probe.
    Live,
    /// Nothing is serving this Instance. Its durable record outlived whatever held it.
    Absent,
    /// This backend cannot tell. It holds machines in the process that launched them, so a
    /// record written by a process that has since exited says nothing either way, and a probe
    /// that guessed would be inventing the one fact this enum exists to avoid inventing.
    Unknown,
}

impl SandboxLiveness {
    /// The stable name a surface reports this by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}
