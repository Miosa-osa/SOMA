//! Attaching one assigned network bundle to a machine, and the link gate over it.
//!
//! Attaching a frame path and permitting traffic along it are deliberately two acts. The TAP
//! arrives before the guest exists as a network peer, so the device holds it with the link
//! down; the link is raised only once the guest has repaired its own interface and the broker
//! has activated the bundle. Collapsing the two would let frames reach a guest whose interface
//! still carries the placeholder identity the Generation was captured with.

use std::os::fd::RawFd;

use super::{NetworkAttachment, SandboxMachine};
use crate::virtio::TapBackend;

impl SandboxMachine {
    /// Gives a machine built without one the frame path and MAC of an assigned bundle.
    ///
    /// The link is left as it was, so this is safe before the vCPU starts and after a restore:
    /// nothing can be received or transmitted until [`Self::set_network_link`] raises it.
    ///
    /// A machine whose Generation declared no network has no device to attach a bundle to, and
    /// this does nothing rather than pretending it did; the caller never had a bundle to give
    /// one, because the same declaration is what decided it would get none.
    pub fn attach_network(&self, network: NetworkAttachment) {
        let NetworkAttachment { tap, mac } = network;
        if let Some(net) = self.shared.lock().net_mut() {
            net.device_mut().attach(Box::new(TapBackend::new(tap)), mac);
        }
    }

    /// Raises or lowers the host-side link gate on the network device.
    ///
    /// Only the host holds this gate: the device offers no status feature, so the guest never
    /// observes it and cannot raise it itself.
    pub fn set_network_link(&self, up: bool) {
        if let Some(net) = self.shared.lock().net_mut() {
            net.device_mut().set_link(up);
        }
        // A link that has just come up may already have frames waiting on the backend, and the
        // descriptor's edge-triggered registration reported those before delivery could place
        // them. Waking the device thread makes it drain what is already there rather than wait
        // for the next arrival.
        if up {
            self.wake_devices();
        }
    }

    /// The host descriptor the device thread watches for inbound frames.
    pub(in crate::x86_64) fn net_backend_fd(&self) -> Option<RawFd> {
        self.shared
            .lock()
            .net_mut()
            .and_then(|net| net.device_mut().backend_fd())
    }
}
