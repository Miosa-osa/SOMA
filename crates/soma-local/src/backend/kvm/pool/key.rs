//! The exact key a prepared machine is prepared for, and the recipe that builds one.
//!
//! Two requests share a pool only when every component matches byte for byte. Nothing is
//! rounded or widened, because a sterile machine prepared under one contract cannot serve a
//! request made under another: the guest has already been told how much memory it has and how
//! large its writable head will be, and those facts are baked into the captured state.

use sha2::{Digest as _, Sha256};
use soma_hostd::PoolKeyDigest;
use soma_kvm::DeviceSet;

use soma_kvm::x86_64::{Hypervisor, SnapshotObjects};
use soma_vmm::sandbox::SterileSpec;

use crate::backend::kvm::prepared::PreparedGeneration;

/// Everything that must match before a prepared machine may serve a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::backend::kvm) struct MachineKey {
    /// The Candidate the snapshot was captured from.
    pub(super) candidate: [u8; 32],
    /// The certified snapshot object identities the machine is restored from.
    pub(super) snapshot: [[u8; 32]; 3],
    /// Guest RAM in bytes.
    pub(super) memory_bytes: u64,
    /// The capacity of the private head this machine will be given.
    pub(super) overlay_capacity_bytes: u64,
    /// The vCPU count.
    ///
    /// The machine contract fixes this at one today. It is still part of the key so that
    /// widening the contract cannot silently let a one-vCPU machine serve a larger request.
    pub(super) vcpus: u16,
    /// The optional devices this machine was built with.
    ///
    /// A pool is keyed on this for the same reason it is keyed on the shape: a machine built
    /// with no overlay device has no slot to attach a private head to, so serving a request
    /// that wants writable storage from that pool would hand back a machine that cannot take
    /// the head it is about to be assigned.
    pub(super) devices: DeviceSet,
}

impl MachineKey {
    /// The digest of the canonical encoding, which names the pool in slot bookkeeping.
    pub(super) fn digest(&self) -> PoolKeyDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"SOMAKVMPOOL");
        hasher.update(self.candidate);
        hasher.update(self.memory_bytes.to_be_bytes());
        hasher.update(self.overlay_capacity_bytes.to_be_bytes());
        hasher.update(self.vcpus.to_be_bytes());
        hasher.update([
            u8::from(self.devices.overlay()),
            u8::from(self.devices.net()),
        ]);
        for digest in self.snapshot {
            hasher.update(digest);
        }
        PoolKeyDigest::from_bytes(hasher.finalize().into())
    }
}

/// The key plus everything needed to open the artifacts one more sterile machine requires.
///
/// The recipe reopens the immutable root for each worker rather than sharing one descriptor,
/// because each machine owns and closes the descriptors it was built from.
pub(in crate::backend::kvm) struct Recipe {
    key: MachineKey,
    prepared: PreparedGeneration,
    root: soma_generation::ArtifactDescriptor,
    memory: soma_generation::ArtifactDescriptor,
    overlay: soma_generation::ArtifactDescriptor,
    state: soma_generation::ArtifactDescriptor,
}

/// Everything [`Recipe::new`] needs to describe one pool and open its artifacts.
pub(in crate::backend::kvm) struct RecipeInputs<'a> {
    /// The admitted Generation whose retained handles supply every restore artifact.
    pub(in crate::backend::kvm) prepared: &'a PreparedGeneration,
    /// The immutable root every Instance of this Generation shares.
    pub(in crate::backend::kvm) root: soma_generation::ArtifactDescriptor,
    pub(in crate::backend::kvm) memory: soma_generation::ArtifactDescriptor,
    pub(in crate::backend::kvm) overlay: soma_generation::ArtifactDescriptor,
    pub(in crate::backend::kvm) state: soma_generation::ArtifactDescriptor,
    /// Guest RAM in bytes.
    pub(in crate::backend::kvm) memory_bytes: u64,
    /// The vCPU count.
    pub(in crate::backend::kvm) vcpus: u16,
    /// The Candidate the snapshot was captured from.
    pub(in crate::backend::kvm) candidate: [u8; 32],
    /// The optional devices machines in this pool are built with.
    pub(in crate::backend::kvm) devices: DeviceSet,
}

impl Recipe {
    /// Describes the pool a request for this snapshot, shape, and head size belongs to.
    ///
    /// Returns `None` when the snapshot carries no sterile overlay template, because the head
    /// capacity is read from that template and a machine cannot be prepared without it.
    pub(in crate::backend::kvm) fn new(inputs: &RecipeInputs) -> Self {
        let RecipeInputs {
            prepared,
            root,
            memory,
            overlay,
            state,
            memory_bytes,
            vcpus,
            candidate,
            devices,
        } = *inputs;
        // A Generation that declared no writable storage published no overlay template, so
        // there is no capacity to read and none is needed: the machine has no overlay device
        // to attach a head to. Only a machine that wants one and cannot find it has no pool.
        let overlay_capacity_bytes = if devices.overlay() { overlay.size } else { 0 };
        Self {
            key: MachineKey {
                candidate,
                snapshot: [
                    *memory.digest.as_bytes(),
                    *overlay.digest.as_bytes(),
                    *state.digest.as_bytes(),
                ],
                memory_bytes,
                overlay_capacity_bytes,
                vcpus,
                devices,
            },
            prepared: prepared.clone(),
            root,
            memory,
            overlay,
            state,
        }
    }

    /// The key this recipe prepares for.
    pub(in crate::backend::kvm) const fn key(&self) -> &MachineKey {
        &self.key
    }

    /// Opens the artifacts for one more sterile machine.
    ///
    /// Returns `None` when the immutable root cannot be opened, which is an operator fault on
    /// the prepared store rather than a fault in any request.
    pub(in crate::backend::kvm) fn spec(&self) -> Option<SterileSpec> {
        let root = self.prepared.open_artifact(&self.root).ok()?;
        let state = self.prepared.open_artifact(&self.state).ok()?;
        let memory = self.prepared.open_artifact(&self.memory).ok()?;
        let overlay = self
            .key
            .devices
            .overlay()
            .then(|| self.prepared.open_artifact(&self.overlay))
            .transpose()
            .ok()?;
        let objects = SnapshotObjects::adopt(state, memory, overlay);
        Some(SterileSpec {
            objects,
            hypervisor: Hypervisor::Device,
            root,
            overlay_capacity_bytes: self
                .key
                .devices
                .overlay()
                .then_some(self.key.overlay_capacity_bytes),
            memory_bytes: self.key.memory_bytes,
            devices: self.key.devices,
        })
    }
}

#[cfg(test)]
mod tests {
    use soma_kvm::DeviceSet;

    use super::MachineKey;

    fn key() -> MachineKey {
        MachineKey {
            candidate: [7; 32],
            snapshot: [[1; 32], [2; 32], [3; 32]],
            memory_bytes: 1 << 30,
            overlay_capacity_bytes: 256 << 20,
            vcpus: 1,
            devices: DeviceSet::new(true, true),
        }
    }

    /// A prepared machine may only serve a request that matches it in every dimension, so every
    /// dimension has to change the digest that names its pool.
    #[test]
    fn every_key_component_changes_the_digest() {
        type Mutation = (&'static str, fn(&mut MachineKey));
        let mutations: [Mutation; 5] = [
            ("candidate", |key| key.candidate = [8; 32]),
            ("snapshot", |key| key.snapshot[0] = [9; 32]),
            ("memory", |key| key.memory_bytes += 4096),
            ("overlay capacity", |key| key.overlay_capacity_bytes += 4096),
            ("vcpus", |key| key.vcpus += 1),
        ];
        let mut seen = vec![key().digest()];
        assert_eq!(key().digest(), seen[0], "the digest is not stable");
        for (component, mutate) in mutations {
            let mut mutated = key();
            mutate(&mut mutated);
            let digest = mutated.digest();
            assert!(
                !seen.contains(&digest),
                "{component} did not change the machine key digest"
            );
            seen.push(digest);
        }
    }
}
