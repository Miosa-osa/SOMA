mod boot;
mod claim;
mod dispatch;
mod evidence;
mod file;
mod held;
mod host;
mod identity;
mod jailed;
mod lifecycle;
mod network;
mod pool;
mod prepared;
mod pty;
mod resolve;
mod runtime;
mod start;

use std::{path::PathBuf, time::Duration};

use soma::BackendKind;
use soma_hostd::{ExhaustedBehavior, Limits};

use crate::{LocalFailure, LocalFailureKind};

use super::clock::OperationClocks;
use host::Role;
use network::BrokerConfiguration;
use pool::{MachineKey, MachinePool};
use runtime::Ownership;
use soma_vmm::sandbox::Session;

pub(crate) use host::host_machine;

/// Names how many sterile machines this host keeps prepared per Generation.
///
/// Preparation costs one stopped virtual machine and its private memory mapping for as long as
/// the runtime lives, which is a policy an operator must be able to size or switch off. A value
/// of zero prepares nothing, so every Launch takes the on-demand path and reports it as such.
const PREPARED_MACHINES: &str = "SOMA_PREPARED_MACHINES";
/// OCI reference whose identity-free machines are restored before hosted traffic begins.
const PREWARM_REFERENCE: &str = "SOMA_PREWARM_REFERENCE";
/// Guest memory shape of the hosted machines restored before traffic begins.
const PREWARM_MEMORY_MIB: &str = "SOMA_PREWARM_MEMORY_MIB";

/// How many machines a resident runtime prepares when the operator names no number.
///
/// A resident runtime can serve another Launch after its current machine is released, so one
/// prepared machine can benefit that later request.
/// A per-Instance machine-host cannot share its pool with any other Launch and is forced to zero
/// separately in [`target_for`].
const DEFAULT_TARGET: usize = 1;

/// How long a claimed machine may stay unassigned, and how long one restore may take.
const CLAIM_DEADLINE: Duration = Duration::from_secs(30);
const CONSTRUCTION_DEADLINE: Duration = Duration::from_secs(60);

/// Prepares hosted processes and, when configured, one identity-free VM inside each process.
pub(crate) fn prewarm_machine_hosts(target: usize) -> Result<(), soma::BackendFailureKind> {
    let plan = match std::env::var(PREWARM_REFERENCE) {
        Ok(reference) if !reference.trim().is_empty() => {
            let memory_mib = std::env::var(PREWARM_MEMORY_MIB)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1024);
            let root = prepared::store_root().ok_or(soma::BackendFailureKind::Unavailable)?;
            let prepared = prepared::find(Some(&root), reference.trim())
                .map_err(|_| soma::BackendFailureKind::Unavailable)?;
            Some(host::PrewarmPlan::new(prepared, memory_mib))
        }
        _ => None,
    };
    host::prewarm_machine_hosts(target, plan.as_ref())
}

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
    /// The one identity-free restored machine a per-Instance host prepared before its request.
    primed: Option<(MachineKey, Session, Option<std::fs::File>)>,
    /// Where this host may build jails, when it was configured to jail its machines.
    ///
    /// It is resolved once rather than per request, because whether a host jails its machines
    /// is a property of the host: a launch that jailed and a later one that did not would be
    /// two different isolation guarantees behind one command.
    jail: Option<jailed::Anchors>,
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
        if host_directory.is_some() {
            prepared::preload()
                .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        }
        Self::with_role(host_directory.map_or(Role::Resident, Role::Hosted))
    }

    /// Opens the Backend for one process that can only ever hold one requested Instance.
    ///
    /// Unlike a reusable resident runtime, this process exits when that Instance is released.
    /// Preparing a spare machine here would consume a second VM that no future request can claim.
    pub(super) fn machine_host() -> Result<Self, LocalFailure> {
        Self::with_role(Role::MachineHost)
    }

    /// Opens one machine host and restores its identity-free machine before accepting a request.
    pub(in crate::backend::kvm) fn primed_machine_host(
        prepared: &prepared::PreparedGeneration,
        memory_mib: u64,
    ) -> Result<Self, LocalFailure> {
        let mut backend = Self::with_role(Role::MachineHost)?;
        if backend.jail.is_some() {
            return Err(LocalFailure::new(LocalFailureKind::BackendUnavailable));
        }
        let recipe = claim::recipe_for(prepared, memory_mib, evidence::CONTRACT_VCPUS)
            .ok_or_else(|| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        let key = recipe.key().clone();
        let spec = recipe
            .spec()
            .ok_or_else(|| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        let session = Session::prepare(spec)
            .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        let overlay = claim::prepared_overlay(prepared)
            .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        backend.primed = Some((key, session, overlay));
        Ok(backend)
    }

    fn with_role(role: Role) -> Result<Self, LocalFailure> {
        let prepared_target = target_for(&role);
        let ownership = Ownership::resolve(Ownership::configured().as_deref())
            .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        let jail = if jailed::Anchors::is_configured() {
            // A host told to jail its machines and unable to build one refuses to open at all.
            // Falling back would hold a live machine in an unjailed process, which is the one
            // outcome this configuration exists to prevent.
            Some(
                jailed::Anchors::configured()
                    .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?,
            )
        } else {
            None
        };
        Ok(Self {
            clocks: OperationClocks::new(),
            role,
            jail,
            ownership,
            live: None,
            primed: None,
            machines: MachinePool::open(limits(prepared_target))
                .or_else(|_| MachinePool::open(limits(0)))
                .expect("a pool that prepares nothing always has valid limits"),
            broker: BrokerConfiguration::from_environment().map(Box::new),
        })
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::LinuxKvm
    }
}

/// The pool target this process can actually make useful.
///
/// One hosted process owns exactly one Instance and exits when that Instance is released.
/// Preparing another full machine there cannot serve a future caller, so doing it only doubles
/// memory pressure during a burst.
/// A resident runtime may outlive one Instance and therefore honors the operator's target.
fn target_for(role: &Role) -> usize {
    match role {
        Role::Resident => configured_target(),
        Role::MachineHost | Role::Hosted(_) => 0,
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

#[cfg(test)]
mod target_tests {
    use super::{Role, target_for};

    #[test]
    fn a_per_instance_host_never_builds_an_unclaimable_spare_machine() {
        assert_eq!(target_for(&Role::Hosted("/run/soma".into())), 0);
        assert_eq!(target_for(&Role::MachineHost), 0);
    }
}
