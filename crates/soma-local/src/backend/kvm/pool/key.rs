//! The exact key a prepared machine is prepared for, and the recipe that builds one.
//!
//! Two requests share a pool only when every component matches byte for byte. Nothing is
//! rounded or widened, because a sterile machine prepared under one contract cannot serve a
//! request made under another: the guest has already been told how much memory it has and how
//! large its writable head will be, and those facts are baked into the captured state.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use soma_hostd::PoolKeyDigest;

use crate::backend::kvm::sterile::SterileSpec;

/// Everything that must match before a prepared machine may serve a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::backend::kvm) struct MachineKey {
    /// The Candidate the snapshot was captured from.
    pub(super) candidate: [u8; 32],
    /// The published snapshot directory the machine is restored from.
    pub(super) snapshot: PathBuf,
    /// Guest RAM in bytes.
    pub(super) memory_bytes: u64,
    /// The capacity of the private head this machine will be given.
    pub(super) overlay_capacity_bytes: u64,
    /// The vCPU count.
    ///
    /// The machine contract fixes this at one today. It is still part of the key so that
    /// widening the contract cannot silently let a one-vCPU machine serve a larger request.
    pub(super) vcpus: u16,
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
        hasher.update(self.snapshot.as_os_str().as_encoded_bytes());
        PoolKeyDigest::from_bytes(hasher.finalize().into())
    }
}

/// The key plus everything needed to open the artifacts one more sterile machine requires.
///
/// The recipe reopens the immutable root for each worker rather than sharing one descriptor,
/// because each machine owns and closes the descriptors it was built from.
pub(in crate::backend::kvm) struct Recipe {
    key: MachineKey,
    store: PathBuf,
    root: soma_generation::ArtifactDescriptor,
}

impl Recipe {
    /// Describes the pool a request for this snapshot, shape, and head size belongs to.
    ///
    /// Returns `None` when the snapshot carries no sterile overlay template, because the head
    /// capacity is read from that template and a machine cannot be prepared without it.
    pub(in crate::backend::kvm) fn new(
        store: &Path,
        root: soma_generation::ArtifactDescriptor,
        snapshot: PathBuf,
        memory_bytes: u64,
        vcpus: u16,
        candidate: [u8; 32],
    ) -> Option<Self> {
        let overlay_capacity_bytes = std::fs::metadata(snapshot.join("overlay.raw")).ok()?.len();
        Some(Self {
            key: MachineKey {
                candidate,
                snapshot,
                memory_bytes,
                overlay_capacity_bytes,
                vcpus,
            },
            store: store.to_path_buf(),
            root,
        })
    }

    /// The key this recipe prepares for.
    pub(in crate::backend::kvm) const fn key(&self) -> &MachineKey {
        &self.key
    }

    /// Opens the artifacts for one more sterile machine.
    ///
    /// Returns `None` when the immutable root cannot be opened, which is an operator fault on
    /// the prepared store rather than a fault in any request.
    pub(super) fn spec(&self) -> Option<SterileSpec> {
        let root = soma_generation::open_artifact(&self.store, &self.root).ok()?;
        Some(SterileSpec {
            snapshot: self.key.snapshot.clone(),
            root,
            overlay_capacity_bytes: self.key.overlay_capacity_bytes,
            memory_bytes: self.key.memory_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::MachineKey;

    fn key() -> MachineKey {
        MachineKey {
            candidate: [7; 32],
            snapshot: "/srv/snap".into(),
            memory_bytes: 1 << 30,
            overlay_capacity_bytes: 256 << 20,
            vcpus: 1,
        }
    }

    /// A prepared machine may only serve a request that matches it in every dimension, so every
    /// dimension has to change the digest that names its pool.
    #[test]
    fn every_key_component_changes_the_digest() {
        type Mutation = (&'static str, fn(&mut MachineKey));
        let mutations: [Mutation; 5] = [
            ("candidate", |key| key.candidate = [8; 32]),
            ("snapshot", |key| key.snapshot = "/srv/other".into()),
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
