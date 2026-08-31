mod boot;
mod claim;
mod evidence;
mod io;
mod lifecycle;
mod pool;
mod prepared;
mod resolve;
mod session;
mod start;
mod sterile;
mod timeline;
mod worker;

use std::time::Duration;

use soma::BackendKind;
use soma_hostd::{ExhaustedBehavior, Limits};

use super::clock::OperationClocks;
use pool::MachinePool;

/// Names how many sterile machines this host keeps prepared per Generation.
///
/// Preparation costs one stopped virtual machine and its private memory mapping for as long as
/// the runtime lives, which is a policy an operator must be able to size or switch off. A value
/// of zero prepares nothing, so every Launch takes the on-demand path and reports it as such.
const PREPARED_MACHINES: &str = "SOMA_PREPARED_MACHINES";

/// How many machines are prepared when the operator names no number.
///
/// One is enough to take machine construction off the next Launch, which is the whole claim
/// being made, and it is the smallest pool that can make it.
const DEFAULT_TARGET: usize = 1;

/// How long a claimed machine may stay unassigned, and how long one restore may take.
const CLAIM_DEADLINE: Duration = Duration::from_secs(30);
const CONSTRUCTION_DEADLINE: Duration = Duration::from_secs(60);

pub(crate) struct KvmBackend {
    clocks: OperationClocks,
    /// The one sandbox this Backend is driving, if any.
    live: Option<lifecycle::Live>,
    /// Machines restored before a request asked for one.
    machines: MachinePool,
}

impl KvmBackend {
    /// Opens the Backend.
    ///
    /// There is nothing to probe. Every artifact a machine needs lives in the store of the
    /// Generation the host prepared, named by digest in its manifest, so a request either finds
    /// a prepared Generation or is refused by name. A Backend that probed the host here would
    /// be asserting something about Generations it has not looked at.
    pub(super) fn open() -> Self {
        Self {
            clocks: OperationClocks::new(),
            live: None,
            machines: MachinePool::open(limits(configured_target()))
                .or_else(|_| MachinePool::open(limits(0)))
                .expect("a pool that prepares nothing always has valid limits"),
        }
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::LinuxKvm
    }
}

/// How many machines this host was told to keep prepared.
///
/// Only a number is accepted. A value that is not one leaves preparation at its default rather
/// than silently switching it off, because a typo must not be the difference between a prepared
/// host and one that quietly stopped preparing.
fn configured_target() -> usize {
    std::env::var_os(PREPARED_MACHINES)
        .and_then(|value| value.to_str().and_then(|text| text.trim().parse().ok()))
        .unwrap_or(DEFAULT_TARGET)
}

/// The bounded policy the pool runs under.
///
/// There is no minimum below which replenishment becomes urgent and no queue when the pool is
/// empty: a Launch that finds nothing prepared restores its own machine and says so, which is a
/// separately measured path rather than a wait.
const fn limits(target: usize) -> Limits {
    Limits {
        min: 0,
        target,
        // The ceiling is never zero even when nothing is prepared, because a pool with no
        // capacity at all is a policy the limits refuse rather than a disabled pool.
        max: if target == 0 { 1 } else { target },
        replenish_concurrency: 1,
        claim_deadline: CLAIM_DEADLINE,
        construction_deadline: CONSTRUCTION_DEADLINE,
        exhausted: ExhaustedBehavior::Reject,
        binding_limit: if target == 0 { 1 } else { target },
    }
}
