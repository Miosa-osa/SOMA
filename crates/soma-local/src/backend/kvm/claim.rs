//! Turning one request into the pool key it belongs to and the authority a claim transfers.
//!
//! The two halves are deliberately separate. The key describes a machine that could serve this
//! request and holds nothing about the request itself, which is what lets it be computed before
//! any Instance exists. The assignment is everything that belongs to exactly one Instance, and
//! it is built on the request path because none of it can be prepared in advance.

use soma::{BackendFailureKind, InstanceId};
use soma_generation::generation_manifest::SnapshotBinding;
use soma_guest::SecretFile;

use super::boot::private_head_from;
use super::evidence::CONTRACT_VCPUS;
use super::identity::{LaunchIdentity, generation_bytes};
use super::pool::{Claimed, MachinePool, Recipe, RecipeInputs};
use super::prepared::PreparedGeneration;
use soma_vmm::sandbox::{Assignment, Network};

/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// The published snapshot directory of a prepared Generation, when it has one.
///
/// A prepared entry holds the Candidate's store beside a snapshot taken once for the whole
/// Generation. Only an entry that carries a captured machine can be restored at all, so an
/// entry without one has no pool and no prepared machine.
pub(super) fn snapshot(
    prepared: &PreparedGeneration,
) -> Option<(
    soma_generation::ArtifactDescriptor,
    soma_generation::ArtifactDescriptor,
    soma_generation::ArtifactDescriptor,
)> {
    match prepared.manifest.snapshot {
        SnapshotBinding::Captured {
            memory,
            overlay,
            state,
            ..
        } => Some((memory, overlay, state)),
        SnapshotBinding::Absent => None,
    }
}

/// The pool a request for this Generation and shape belongs to, and how to fill it.
///
/// Returns `None` when the entry carries no snapshot or no immutable root, because neither a
/// prepared machine nor an on-demand restore exists for it and there is nothing to pool.
fn recipe_for(prepared: &PreparedGeneration, memory_mib: u64, vcpus: u16) -> Option<Recipe> {
    let (memory, overlay, state) = snapshot(prepared)?;
    let devices = prepared.manifest.device_set();
    let root = prepared.manifest.root.descriptor;
    let candidate = generation_bytes(&prepared.id).ok()?;
    Some(Recipe::new(&RecipeInputs {
        store: &prepared.store,
        root,
        memory,
        overlay,
        state,
        memory_bytes: memory_mib * MIB,
        vcpus,
        candidate,
        devices,
    }))
}

/// The fresh authority one claimed machine receives, exactly once.
///
/// The private head is cloned here rather than in the pool: it is this Instance's disk, and a
/// prepared machine that already held one would not be sterile. The head is cloned from the
/// snapshot's own quiesced overlay template rather than the Candidate's untouched one, because
/// the captured machine has already written to it.
pub(super) fn assignment_for(
    prepared: &PreparedGeneration,
    identity: LaunchIdentity,
    network: Network,
    secrets: Vec<SecretFile>,
) -> Result<Assignment, BackendFailureKind> {
    let instance = InstanceId::new(hex(identity.instance))
        .map_err(|_| BackendFailureKind::WorkloadRejected)?;
    // A machine built with no overlay device has no slot to attach a head to, so cloning one
    // here would produce a private disk nothing could mount.
    let overlay = prepared
        .manifest
        .device_set()
        .overlay()
        .then(|| {
            let (_, overlay, _) = snapshot(prepared).ok_or(BackendFailureKind::Unavailable)?;
            let file = soma_generation::open_artifact(&prepared.store, &overlay)
                .map_err(|_| BackendFailureKind::Unavailable)?;
            private_head_from(file, &instance)
        })
        .transpose()?;
    Ok(Assignment {
        overlay,
        generation: generation_bytes(&prepared.id)?,
        instance: identity.instance,
        operation: identity.operation,
        guest_cid: identity.guest_cid,
        network,
        secrets,
    })
}

/// The Instance identity as the lowercase hexadecimal its portable form is written in.
fn hex(instance: [u8; 16]) -> String {
    use std::fmt::Write as _;
    instance
        .iter()
        .fold(String::with_capacity(32), |mut out, byte| {
            let _ignored = write!(out, "{byte:02x}");
            out
        })
}

/// One machine claimed from the pool, with the snapshot its private head must be cloned from.
pub(super) struct ClaimedMachine {
    /// The claimed machine, which must be assigned or destroyed.
    pub(super) machine: Claimed,
}

/// Registers this Generation with the pool, then claims a machine prepared for it.
///
/// Registration and the claim are one step because the request path is the only place that
/// learns which Generation and shape this host is being asked for. Registering does not
/// construct anything on this path: it names the key and wakes the replenisher, which builds on
/// its own thread, so the first request for a Generation finds nothing prepared and a later one
/// does.
///
/// Returns `None` when the entry cannot be pooled at all or when the pool is empty. Neither is
/// a failure, and neither may be reported as a prepared launch.
pub(super) fn prepare_and_claim(
    pool: &MachinePool,
    prepared: &PreparedGeneration,
    memory_mib: u64,
) -> Option<ClaimedMachine> {
    let recipe = recipe_for(prepared, memory_mib, CONTRACT_VCPUS)?;
    let key = recipe.key().clone();
    pool.serve(recipe);
    pool.claim(&key).map(|machine| ClaimedMachine { machine })
}
