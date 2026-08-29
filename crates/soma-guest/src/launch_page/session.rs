use core::fmt;

use crate::{
    Error, InitiatorAwaitingResponse, InitiatorHandshake, InstancePsk, ResponderHandshake,
    ResponderPendingResponse, ResponderPrivateKey, ResponderPublicKey, SessionBinding,
};

/// Host launch secrets after one delivery callback reported success.
///
/// Connecting a host owner consumes this state, so one reported delivery enables one attempt.
///
/// ```compile_fail
/// use soma_guest::{HostLaunchMaterial, LaunchNetwork};
///
/// let network = LaunchNetwork::new(3, 1, [2, 0, 0, 0, 0, 1], [10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 0, 1], 1).unwrap();
/// let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], network).unwrap();
/// let delivered = material.deliver_with(|_| Ok::<(), ()>(())).unwrap();
/// let _started = delivered.start_initiator();
/// let _reused = delivered.binding();
/// ```
pub struct DeliveredHostLaunchMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
    responder: ResponderPublicKey,
}

/// Guest session material available only after the caller repairs entropy.
///
/// Connecting a guest owner consumes this state, so one injected PSK authorizes one handshake.
///
/// ```compile_fail
/// use soma_guest::{GuestLaunchMaterial, HostLaunchMaterial, LAUNCH_PAGE_SIZE, LaunchNetwork};
///
/// let network = LaunchNetwork::new(3, 1, [2, 0, 0, 0, 0, 1], [10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 0, 1], 1).unwrap();
/// let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], network).unwrap();
/// let mut page = [0; LAUNCH_PAGE_SIZE];
/// let host = host.deliver_with(|bytes| { page.copy_from_slice(bytes); Ok::<(), ()>(()) }).unwrap();
/// let guest = GuestLaunchMaterial::take_from_page(&mut page).unwrap();
/// let guest = guest.reseed_with(|_| Ok::<(), ()>(())).unwrap();
/// let (_, first) = host.start_initiator().unwrap();
/// let _pending = guest.start_responder(&first);
/// let _reused = guest.start_responder(&first);
/// ```
pub struct GuestSessionMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
    responder: ResponderPrivateKey,
}

impl DeliveredHostLaunchMaterial {
    pub(super) const fn new(
        binding: SessionBinding,
        psk: InstancePsk,
        responder: ResponderPublicKey,
    ) -> Self {
        Self {
            binding,
            psk,
            responder,
        }
    }

    /// Returns the fresh public responder identity this Instance's guest must prove.
    #[must_use]
    pub const fn responder_public_key(&self) -> &ResponderPublicKey {
        &self.responder
    }

    /// Borrows the exact transcript binding delivered to the guest.
    #[must_use]
    pub const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Consumes delivered material to start exactly one authenticated host handshake.
    ///
    /// # Errors
    ///
    /// Returns a redacted cryptographic setup error.
    pub(crate) fn start_initiator(self) -> Result<(InitiatorAwaitingResponse, Vec<u8>), Error> {
        InitiatorHandshake::start(&self.binding, &self.responder, self.psk)
    }
}

impl GuestSessionMaterial {
    pub(super) const fn new(
        binding: SessionBinding,
        psk: InstancePsk,
        responder: ResponderPrivateKey,
    ) -> Self {
        Self {
            binding,
            psk,
            responder,
        }
    }

    pub(crate) const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Starts the authenticated guest handshake after entropy repair.
    ///
    /// # Errors
    ///
    /// Returns a redacted authentication or setup error.
    pub(crate) fn start_responder(self, first: &[u8]) -> Result<ResponderPendingResponse, Error> {
        ResponderHandshake::accept(&self.binding, &self.responder, self.psk, first)
    }
}

macro_rules! redacted_debug {
    ($type:ty, $name:literal) => {
        impl fmt::Debug for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($name, "([REDACTED])"))
            }
        }
    };
}

redacted_debug!(DeliveredHostLaunchMaterial, "DeliveredHostLaunchMaterial");
redacted_debug!(GuestSessionMaterial, "GuestSessionMaterial");
