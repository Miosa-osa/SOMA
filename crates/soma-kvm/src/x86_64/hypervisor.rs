//! Where a machine's `/dev/kvm` handle comes from.
//!
//! A host process opens the device by path. A jailed machine cannot: the character device is
//! not in its empty root, and the jail's descriptor manifest names an already-open `/dev/kvm`
//! precisely so that it never has to be. Both arrive at the same [`Kvm`] handle, and nothing
//! downstream can tell which side opened it.

use std::os::fd::{FromRawFd as _, IntoRawFd as _, OwnedFd};

use kvm_ioctls::Kvm;

/// How this machine obtains its hypervisor handle.
#[derive(Debug)]
pub enum Hypervisor {
    /// Open `/dev/kvm` by path, which is what an unjailed host process does.
    Device,
    /// Adopt a `/dev/kvm` handle a broker already opened and transferred.
    Adopted(OwnedFd),
}

impl Hypervisor {
    /// Produces the hypervisor handle.
    ///
    /// # Errors
    ///
    /// Returns the open failure of `/dev/kvm`; adopting a transferred descriptor cannot fail,
    /// because the descriptor's kind was verified by the sealed-table attestation before the
    /// machine was asked for anything.
    #[allow(unsafe_code)]
    pub fn handle(self) -> Result<Kvm, kvm_ioctls::Error> {
        match self {
            Self::Device => Kvm::new(),
            // SAFETY: the descriptor is owned here and is given up to the returned handle, so
            // exactly one owner closes it.
            Self::Adopted(descriptor) => Ok(unsafe { Kvm::from_raw_fd(descriptor.into_raw_fd()) }),
        }
    }
}

impl Default for Hypervisor {
    /// A caller that names no source opens the device, which is what every host path does.
    fn default() -> Self {
        Self::Device
    }
}
