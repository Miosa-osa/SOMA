//! The `Server-Timing` header every runner answer carries (contract C2).

use std::{fmt::Write as _, time::Duration};

use soma::{ExecutionReceipt, MilestoneKind};

use crate::runner::backend::CallTiming;

/// Where each request's time went, as `Server-Timing` reports it in milliseconds.
///
/// `auth`, `pool` and `exec` are the segments the header has always carried, and
/// `exec` keeps meaning the whole facade call so a client that reads it today
/// keeps working. The rest decompose that call and the bookkeeping the runner
/// does around it, so the header accounts for the entire server-side request
/// instead of leaving part of it unnamed.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Timing {
    pub(super) auth: Duration,
    pub(super) call: CallTiming,
    /// The runner's own create checks and bookkeeping, before the facade call:
    /// the parameters, admission, the tenant's share, minting the id, reserving
    /// the sandbox, and building the launch request.
    pub(super) prep: Duration,
    /// The launch evidence's own phases, which `soma` times and records itself.
    pub(super) launch: LaunchPhases,
    /// The runner's own bookkeeping after the facade call returned: confirming
    /// the sandbox and encoding the `201` body.
    pub(super) finish: Duration,
}

/// The phase boundaries `soma` already records, read back from a receipt.
///
/// The engine times these itself and writes them as receipt milestones, so the
/// runner reports evidence that was already being produced rather than adding
/// instrumentation to the launch path. Each field is the span between two
/// consecutive milestones; together they sum to at most the facade call the
/// `exec` segment reports.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LaunchPhases {
    /// Accepted to `workload_resolved`: resolving the image into a workload.
    pub(super) resolve: Duration,
    /// `workload_resolved` to `admitted`: the write-ahead intent the state store
    /// durably records before anything is launched.
    pub(super) admit: Duration,
    /// `admitted` to `machine_launched`: claiming a prepared machine, assigning it
    /// its identity, and writing its launch page. The network bundle claim and
    /// activation also happen inside this span for a sandbox that asked for a
    /// network, and the host protocol does not separate them, so they are not
    /// broken out here.
    pub(super) assign: Duration,
    /// `machine_launched` to `ready`: starting the vCPU and waiting for the guest
    /// agent's authenticated handshake.
    pub(super) ready: Duration,
    /// `ready` to the end of the facade call: the evidence the engine writes after
    /// the guest is up, including the second durable state-store write. It is the
    /// remainder the receipt's milestones do not name, so it is derived by
    /// subtracting the receipt's `ready` from `exec` rather than measured, and it
    /// saturates at zero if the two clocks disagree.
    pub(super) commit: Duration,
}

impl Timing {
    pub(super) fn header(self) -> String {
        let mut value = String::with_capacity(128);
        for (index, (name, duration)) in [
            ("auth", self.auth),
            ("pool", self.call.pool),
            ("exec", self.call.exec),
            ("prep", self.prep),
            ("resolve", self.launch.resolve),
            ("admit", self.launch.admit),
            ("assign", self.launch.assign),
            ("ready", self.launch.ready),
            ("commit", self.launch.commit),
            ("finish", self.finish),
        ]
        .into_iter()
        .enumerate()
        {
            if index > 0 {
                value.push(',');
            }
            let micros = duration.as_micros();
            let _written = write!(value, "{name};dur={}.{:03}", micros / 1_000, micros % 1_000);
        }
        value
    }
}

impl LaunchPhases {
    /// Reads the phase boundaries out of a receipt's milestones.
    ///
    /// A milestone the receipt does not carry leaves its phase at zero, and a
    /// milestone that appears more than once keeps its last value, which is what
    /// a replayed or retried operation produces.
    pub(super) fn from_receipt(receipt: &ExecutionReceipt, exec: Duration) -> Self {
        let mut accepted = None;
        let mut resolved = None;
        let mut admitted = None;
        let mut launched = None;
        let mut ready = None;
        for milestone in receipt.milestones() {
            let at = Duration::from_nanos(milestone.elapsed_ns());
            match milestone.kind() {
                MilestoneKind::Accepted => accepted = Some(at),
                MilestoneKind::WorkloadResolved => resolved = Some(at),
                MilestoneKind::Admitted => admitted = Some(at),
                MilestoneKind::MachineLaunched => launched = Some(at),
                MilestoneKind::Ready => ready = Some(at),
                _ => {}
            }
        }
        Self {
            resolve: between(accepted, resolved),
            admit: between(resolved, admitted),
            assign: between(admitted, launched),
            ready: between(launched, ready),
            commit: match ready {
                Some(ready) => exec.saturating_sub(ready),
                None => Duration::ZERO,
            },
        }
    }
}

/// The span between two milestones, or zero when either is missing.
fn between(from: Option<Duration>, to: Option<Duration>) -> Duration {
    match (from, to) {
        (Some(from), Some(to)) => to.saturating_sub(from),
        _ => Duration::ZERO,
    }
}

/// Whole milliseconds, as the journal and the usage fields carry them.
pub(super) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "timing_tests.rs"]
mod tests;
