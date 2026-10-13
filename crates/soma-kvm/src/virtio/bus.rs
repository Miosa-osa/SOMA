//! The MMIO bus: checked interval dispatch over the seven fixed transports.
//!
//! The bus owns the transports and routes reads, writes, notifications,
//! inbound delivery, and snapshot records by slot. It performs no KVM call;
//! ioeventfd and irqfd registration are the [`NotifySource`] and
//! [`IrqSink`] seams the machine layer implements.

pub mod access;
pub mod slots;
pub mod table;

use crate::virtio::devices::block::{BlockDevice, BlockRole};
use crate::virtio::devices::mem::MemDevice;
use crate::virtio::devices::net::NetDevice;
use crate::virtio::devices::rng::RngDevice;
use crate::virtio::devices::vsock::VsockDevice;
use crate::virtio::guest_memory::{GuestAddress, GuestMemory};
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::registers::AccessWidth;
pub use access::{BusConfigError, BusEvent, BusViolation, IrqSink, NotifySource};
pub use table::{DeviceSet, FIRST_GSI, MMIO_WINDOW_BASE, SLOT_COUNT, Slot, kernel_command_line};

/// The device models in slot order, before they are bound to transports.
///
/// The three optional slots are `None` when the Generation declared none of the capability they
/// carry, which is what keeps a read-only, network-free, or fixed-size machine from paying for a
/// device it would never use.
pub struct BusDevices {
    pub root: BlockDevice,
    pub overlay: Option<BlockDevice>,
    pub volume: Option<BlockDevice>,
    pub net: Option<NetDevice>,
    pub vsock: VsockDevice,
    pub rng: RngDevice,
    pub mem: Option<MemDevice>,
}

impl BusDevices {
    /// The set these models make up.
    #[must_use]
    pub const fn device_set(&self) -> DeviceSet {
        DeviceSet::new(self.overlay.is_some(), self.net.is_some())
            .with_mem(self.mem.is_some())
            .with_volume(self.volume.is_some())
    }
}

/// The present transports at their fixed pages.
pub struct MmioBus {
    root: MmioTransport<BlockDevice>,
    overlay: Option<MmioTransport<BlockDevice>>,
    volume: Option<MmioTransport<BlockDevice>>,
    net: Option<MmioTransport<NetDevice>>,
    vsock: MmioTransport<VsockDevice>,
    rng: MmioTransport<RngDevice>,
    mem: Option<MmioTransport<MemDevice>>,
}

/// Runs `$body` against the slot's transport, or yields `None` when the slot is absent.
///
/// Every caller has to decide what an absent slot means for it, so the macro refuses to guess:
/// a bus access rejects the address, a service pass reports nothing done, and a capture writes
/// the absent record. Yielding an `Option` is what forces each of those to be written down.
macro_rules! with_slot {
    ($bus:expr, $slot:expr, |$t:ident| $body:expr) => {
        match $slot {
            Slot::Root => {
                let $t = &mut $bus.root;
                Some($body)
            }
            Slot::Overlay => $bus.overlay.as_mut().map(|$t| $body),
            Slot::Net => $bus.net.as_mut().map(|$t| $body),
            Slot::Vsock => {
                let $t = &mut $bus.vsock;
                Some($body)
            }
            Slot::Rng => {
                let $t = &mut $bus.rng;
                Some($body)
            }
            Slot::Mem => $bus.mem.as_mut().map(|$t| $body),
            Slot::Volume => $bus.volume.as_mut().map(|$t| $body),
        }
    };
}
pub(super) use with_slot;

impl MmioBus {
    /// Binds the present device models to fresh transports.
    ///
    /// # Errors
    /// Rejects a block device in the wrong slot or a transport misconfiguration.
    pub fn new(devices: BusDevices) -> Result<Self, BusConfigError> {
        check_role(Slot::Root, &devices.root, BlockRole::ImmutableRoot)?;
        if let Some(overlay) = &devices.overlay {
            check_role(Slot::Overlay, overlay, BlockRole::PrivateOverlay)?;
        }
        if let Some(volume) = &devices.volume {
            check_role(Slot::Volume, volume, BlockRole::DurableVolume)?;
        }
        Ok(Self {
            root: transport(Slot::Root, devices.root)?,
            overlay: devices
                .overlay
                .map(|device| transport(Slot::Overlay, device))
                .transpose()?,
            volume: devices
                .volume
                .map(|device| transport(Slot::Volume, device))
                .transpose()?,
            net: devices
                .net
                .map(|device| transport(Slot::Net, device))
                .transpose()?,
            vsock: transport(Slot::Vsock, devices.vsock)?,
            rng: transport(Slot::Rng, devices.rng)?,
            mem: devices
                .mem
                .map(|device| transport(Slot::Mem, device))
                .transpose()?,
        })
    }

    /// Which optional slots this bus actually carries.
    #[must_use]
    pub const fn device_set(&self) -> DeviceSet {
        DeviceSet::new(self.overlay.is_some(), self.net.is_some())
            .with_mem(self.mem.is_some())
            .with_volume(self.volume.is_some())
    }

    /// Reads from a transport register or configuration byte range.
    ///
    /// # Errors
    /// Returns the typed rejection; the machine decides the observed value.
    pub fn dispatch_read(&mut self, gpa: u64, width: AccessWidth) -> Result<u64, BusViolation> {
        let (slot, offset) = Slot::from_gpa(gpa).ok_or(BusViolation::UnmappedAddress { gpa })?;
        // An absent slot's page is reserved but holds nothing, so it is unmapped rather than a
        // device that answers with zeros: a guest that was never told the page exists and reads
        // it anyway is doing something the machine should treat as fatal, not humour.
        with_slot!(self, slot, |t| t.read(offset, width))
            .ok_or(BusViolation::UnmappedAddress { gpa })?
            .map_err(|violation| BusViolation::Transport { slot, violation })
    }

    /// Applies a driver write.
    ///
    /// # Errors
    /// Returns the typed rejection; an accepted write never partially mutates state.
    pub fn dispatch_write<M: GuestMemory + ?Sized>(
        &mut self,
        gpa: u64,
        width: AccessWidth,
        value: u64,
        mem: &M,
    ) -> Result<BusEvent, BusViolation> {
        let (slot, offset) = Slot::from_gpa(gpa).ok_or(BusViolation::UnmappedAddress { gpa })?;
        let event = with_slot!(self, slot, |t| t.write(offset, width, value, mem))
            .ok_or(BusViolation::UnmappedAddress { gpa })?
            .map_err(|violation| BusViolation::Transport { slot, violation })?;
        Ok(BusEvent { slot, event })
    }

    /// Whether the slot has unacknowledged interrupt status.
    #[must_use]
    pub fn interrupt_pending(&self, slot: Slot) -> bool {
        self.interrupt_status(slot) != 0
    }

    /// Raw interrupt status of a slot; an absent slot can never have one pending.
    #[must_use]
    pub fn interrupt_status(&self, slot: Slot) -> u32 {
        match slot {
            Slot::Root => self.root.interrupt_status(),
            Slot::Overlay => self
                .overlay
                .as_ref()
                .map_or(0, MmioTransport::interrupt_status),
            Slot::Net => self.net.as_ref().map_or(0, MmioTransport::interrupt_status),
            Slot::Vsock => self.vsock.interrupt_status(),
            Slot::Rng => self.rng.interrupt_status(),
            Slot::Mem => self.mem.as_ref().map_or(0, MmioTransport::interrupt_status),
            Slot::Volume => self
                .volume
                .as_ref()
                .map_or(0, MmioTransport::interrupt_status),
        }
    }

    /// Signals the slot's GSI through the sink.
    ///
    /// # Errors
    /// Forwards the sink's failure.
    pub fn signal<S: IrqSink>(&self, slot: Slot, sink: &mut S) -> Result<(), S::Error> {
        sink.signal(slot.gsi())
    }

    /// Registers every `(slot, queue)` notification address with the source.
    ///
    /// # Errors
    /// Forwards the source's failure.
    pub fn register_notify_sources<N: NotifySource>(&self, source: &mut N) -> Result<(), N::Error> {
        for slot in self.device_set().present() {
            for queue in 0..slot.queue_count() {
                source.register(GuestAddress(slot.notify_addr()), slot, queue)?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn root(&self) -> &MmioTransport<BlockDevice> {
        &self.root
    }

    #[must_use]
    pub const fn overlay(&self) -> Option<&MmioTransport<BlockDevice>> {
        self.overlay.as_ref()
    }

    #[must_use]
    pub const fn net(&self) -> Option<&MmioTransport<NetDevice>> {
        self.net.as_ref()
    }

    pub const fn net_mut(&mut self) -> Option<&mut MmioTransport<NetDevice>> {
        self.net.as_mut()
    }

    #[must_use]
    pub const fn vsock(&self) -> &MmioTransport<VsockDevice> {
        &self.vsock
    }

    pub const fn vsock_mut(&mut self) -> &mut MmioTransport<VsockDevice> {
        &mut self.vsock
    }

    /// The private overlay, so a claimed worker can be given the disk head it was built without.
    pub const fn overlay_mut(&mut self) -> Option<&mut MmioTransport<BlockDevice>> {
        self.overlay.as_mut()
    }

    /// The durable data volume of ADR 0053 Step A, present only on a persistent Generation.
    #[must_use]
    pub const fn volume(&self) -> Option<&MmioTransport<BlockDevice>> {
        self.volume.as_ref()
    }

    /// The durable data volume, so a launch can bind the real file over the placeholder.
    pub const fn volume_mut(&mut self) -> Option<&mut MmioTransport<BlockDevice>> {
        self.volume.as_mut()
    }

    #[must_use]
    pub const fn rng(&self) -> &MmioTransport<RngDevice> {
        &self.rng
    }

    /// The memory device's transport, when the machine can be resized.
    #[must_use]
    pub const fn mem(&self) -> Option<&MmioTransport<MemDevice>> {
        self.mem.as_ref()
    }

    /// The memory device's transport, so the host can raise or lower the requested size and
    /// signal the configuration change.
    pub const fn mem_mut(&mut self) -> Option<&mut MmioTransport<MemDevice>> {
        self.mem.as_mut()
    }
}

/// The bus moves to the device thread and is shared with the vCPU thread behind a mutex, so
/// every device model and backend must be `Send`; this fails to compile otherwise.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<MmioBus>();
    assert_send::<BusDevices>();
};

fn check_role(slot: Slot, device: &BlockDevice, expected: BlockRole) -> Result<(), BusConfigError> {
    if device.role() == expected {
        Ok(())
    } else {
        Err(BusConfigError::BlockRole {
            slot,
            role: device.role(),
        })
    }
}

fn transport<D: crate::virtio::device::VirtioDevice>(
    slot: Slot,
    device: D,
) -> Result<MmioTransport<D>, BusConfigError> {
    MmioTransport::new(device).map_err(|error| BusConfigError::Transport { slot, error })
}

#[cfg(test)]
mod tests;
