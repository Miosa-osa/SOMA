//! Putting a real frame path behind a device that was built without one.
//!
//! A network bundle is per-Instance authority: it holds a namespace, a TAP descriptor, an
//! address lease, and a MAC, and the prepared worker protocol transfers it when a worker is
//! claimed. A prepared worker therefore cannot hold one, and its network device has to exist
//! before the bundle does, exactly as its overlay device does.
//!
//! So the device is built against the loopback backend, which drops every frame while the link
//! is down, and the assigned TAP replaces it here. The MAC arrives with the same bundle and is
//! set in the same step, because a device serving one Instance's frames under another
//! Instance's address would be the whole failure this ordering exists to prevent.
//!
//! The link stays down. Raising it is a separate admitted step that happens after the guest has
//! repaired its own interface, so attaching a frame path is not the same act as permitting
//! traffic to flow along it.

use super::{NetBackend, NetDevice};

impl NetDevice {
    /// Gives an unattached device the frame path and MAC of one assigned bundle.
    ///
    /// The link is deliberately left as it was. A caller raises it once the guest's interface is
    /// repaired and the network is activated.
    pub fn attach(&mut self, backend: Box<dyn NetBackend + Send>, mac: [u8; 6]) {
        self.backend = backend;
        self.set_mac(mac);
    }
}
