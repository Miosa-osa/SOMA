//! Everything a jailed machine is built from, taken out of its sealed descriptor table.
//!
//! The worker has no filesystem, so it opens nothing. The broker outside the jail resolved the
//! Generation store, opened the hypervisor device and the immutable artifacts, cloned this
//! Instance's private head and unlinked it, and sealed all of it into the manifest slots the
//! launcher fixed before `execveat`. This module is the other end of that: it turns those slot
//! numbers into owned handles exactly once.
//!
//! The slots are not verified here. The worker attested the whole sealed table, including each
//! slot's file type, before it served anything, and refuses to run at all when that attestation
//! does not describe a jail.

#![allow(unsafe_code)]

use std::fs::File;
use std::os::fd::{FromRawFd as _, OwnedFd};

use soma_jail::{ArtifactKind, DescriptorManifest, DescriptorRole};

/// The open handles one jailed machine is built from.
pub(crate) struct MachineResources {
    pub(crate) kvm: OwnedFd,
    /// The published snapshot's state manifest.
    pub(crate) state: File,
    /// The published snapshot's memory image.
    pub(crate) memory: File,
    /// The immutable root every Instance of this Generation shares.
    pub(crate) root: File,
    /// This Instance's private writable head, absent for a Generation with no writable storage.
    pub(crate) overlay: Option<File>,
}

impl MachineResources {
    /// Adopts the manifest slots this machine needs, or `None` when one of them is absent.
    ///
    /// The overlay is the only optional role: a Generation that declared no writable storage
    /// has no head, and the launch that names one is refused by the restore rather than here.
    pub(crate) fn adopt(manifest: &DescriptorManifest) -> Option<Self> {
        let kvm = own(manifest, DescriptorRole::Kvm)?;
        let state = own(
            manifest,
            DescriptorRole::Artifact(ArtifactKind::DeviceState),
        )?;
        let memory = own(
            manifest,
            DescriptorRole::Artifact(ArtifactKind::MemorySnapshot),
        )?;
        let root = own(manifest, DescriptorRole::RootDisk)?;
        let overlay = own(manifest, DescriptorRole::OverlayHead);
        Some(Self {
            kvm,
            state: File::from(state),
            memory: File::from(memory),
            root: File::from(root),
            overlay: overlay.map(File::from),
        })
    }

    /// The capacity the private head has, which the restore builds the overlay slot against.
    pub(crate) fn overlay_capacity_bytes(&self) -> Option<u64> {
        self.overlay
            .as_ref()
            .and_then(|head| head.metadata().ok())
            .map(|metadata| metadata.len())
    }
}

/// Takes ownership of one manifest slot.
fn own(manifest: &DescriptorManifest, role: DescriptorRole) -> Option<OwnedFd> {
    let slot = manifest.slot_for(role)?;
    let descriptor = libc::c_int::try_from(slot).ok()?;
    // SAFETY: the launcher sealed this slot and nothing else in this process owns it; each
    // role appears at most once in a validated manifest, so it is adopted exactly once.
    Some(unsafe { OwnedFd::from_raw_fd(descriptor) })
}
