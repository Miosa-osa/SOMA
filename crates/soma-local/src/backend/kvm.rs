mod boot;
mod claim;
mod dispatch;
mod evidence;
mod file;
mod host;
mod identity;
mod io;
mod lifecycle;
mod network;
mod pool;
mod prepared;
mod pty;
mod resolve;
mod runtime;
mod secrets;
mod session;
mod start;
mod sterile;
mod timeline;
mod worker;

use std::{path::PathBuf, time::Duration};

use soma::BackendKind;
use soma_hostd::{ExhaustedBehavior, Limits};

use crate::{LocalFailure, LocalFailureKind};

use super::clock::OperationClocks;
use host::Role;
use network::BrokerConfiguration;
use pool::MachinePool;
use runtime::Ownership;

pub(crate) use host::host_machine;

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
    /// Which process holds the machines this Backend launches.
    role: Role,
    /// Who owns the Instances this Backend launches.
    ownership: Ownership,
    /// The one sandbox this Backend is driving, if any.
    live: Option<lifecycle::Live>,
    /// Machines restored before a request asked for one.
    machines: MachinePool,
    /// The privileged network broker this host is configured to reach, if it has one.
    ///
    /// This is read once rather than per request: whether a host has a broker is a property of
    /// the host, and a launch that found one and a later launch that did not would otherwise
    /// disagree about what this Backend can serve.
    broker: Option<Box<BrokerConfiguration>>,
}

impl KvmBackend {
    /// Opens the Backend for a caller whose machines must outlive its process.
    ///
    /// `host_directory` names where hosted machines are addressed. A caller that supplies one is
    /// asking for a machine it can reach again from a later process, so every Launch starts a
    /// host that holds it. A caller that supplies none keeps the one-shot lifecycle, in which
    /// the machine lives in this process and dies with it, which is what `soma run` is.
    ///
    /// There is nothing else to probe. Every artifact a machine needs lives in the store of the
    /// Generation the host prepared, named by digest in its manifest, so a request either finds
    /// a prepared Generation or is refused by name. A Backend that probed the host here would
    /// be asserting something about Generations it has not looked at.
    ///
    /// The one thing that is resolved here is who owns Instances. A configured Host Runtime
    /// that cannot be reached refuses the Backend rather than degrading to the one-shot
    /// lifecycle, because an operator who asked for persistent ownership must not silently
    /// receive an Instance that dies with this process.
    ///
    /// # Errors
    ///
    /// Returns [`LocalFailureKind::BackendUnavailable`] when a Host Runtime is configured and
    /// nothing serves it.
    pub(super) fn open(host_directory: Option<PathBuf>) -> Result<Self, LocalFailure> {
        Self::with_role(host_directory.map_or(Role::Resident, Role::Hosted))
    }

    /// Opens the Backend for the process that will hold the machine itself.
    ///
    /// # Errors
    ///
    /// Returns [`LocalFailureKind::BackendUnavailable`] when a Host Runtime is configured and
    /// nothing serves it.
    pub(super) fn resident() -> Result<Self, LocalFailure> {
        Self::with_role(Role::Resident)
    }

    fn with_role(role: Role) -> Result<Self, LocalFailure> {
        let ownership = Ownership::resolve(Ownership::configured().as_deref())
            .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        Ok(Self {
            clocks: OperationClocks::new(),
            role,
            ownership,
            live: None,
            machines: MachinePool::open(limits(configured_target()))
                .or_else(|_| MachinePool::open(limits(0)))
                .expect("a pool that prepares nothing always has valid limits"),
            broker: BrokerConfiguration::from_environment().map(Box::new),
        })
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
