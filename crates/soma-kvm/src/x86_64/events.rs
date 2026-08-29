//! The eventfd seams between the virtio bus and KVM: one irqfd per slot, one ioeventfd per
//! queue-notify address.
//!
//! Registration happens before the vCPU can run and every descriptor is deregistered and
//! closed in `Drop`, so a failed or timed-out sandbox leaves no KVM route behind.

use kvm_ioctls::{IoEventAddress, VmFd};
use vmm_sys_util::eventfd::EventFd;

use super::error::{MachineError, Phase};
use crate::virtio::{FIRST_GSI, GuestAddress, IrqSink, MmioBus, NotifySource, SLOT_COUNT, Slot};

/// The five device interrupt lines, one edge-triggered irqfd per slot on GSIs 5 through 9.
///
/// KVM's in-kernel irqchip creates default routing for GSIs 0 through 23 (PIC and IOAPIC
/// for 0 through 15), so no `KVM_SET_GSI_ROUTING` call is needed for these lines; the
/// pinned guest boots with `noapic` and services them through the PIC.
pub(crate) struct IrqLines {
    lines: Vec<EventFd>,
    registered: bool,
}

impl IrqLines {
    pub(crate) fn create() -> Result<Self, MachineError> {
        let mut lines = Vec::with_capacity(SLOT_COUNT);
        for _ in Slot::ALL {
            lines.push(
                EventFd::new(libc::EFD_NONBLOCK)
                    .map_err(|error| MachineError::io(Phase::Events, &error))?,
            );
        }
        Ok(Self {
            lines,
            registered: false,
        })
    }

    pub(crate) fn register(&mut self, vm: &VmFd) -> Result<(), MachineError> {
        for (line, slot) in self.lines.iter().zip(Slot::ALL) {
            vm.register_irqfd(line, slot.gsi())
                .map_err(|error| MachineError::os(Phase::Events, error))?;
        }
        self.registered = true;
        Ok(())
    }

    pub(crate) fn unregister(&mut self, vm: &VmFd) {
        if self.registered {
            for (line, slot) in self.lines.iter().zip(Slot::ALL) {
                let _ignored = vm.unregister_irqfd(line, slot.gsi());
            }
            self.registered = false;
        }
    }

    /// Signals one edge on the slot's line; a failed eventfd write is reported, not fatal.
    pub(crate) fn signal_slot(&self, slot: Slot) -> bool {
        self.lines
            .get(usize::from(slot.index()))
            .is_some_and(|line| line.write(1).is_ok())
    }
}

impl IrqSink for IrqLines {
    type Error = MachineError;

    fn signal(&mut self, gsi: u32) -> Result<(), MachineError> {
        let index = gsi
            .checked_sub(FIRST_GSI)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.lines.get(index))
            .ok_or_else(|| MachineError::invalid(Phase::Events, "GSI outside the device table"))?;
        index
            .write(1)
            .map_err(|error| MachineError::io(Phase::Events, &error))
    }
}

/// One registered queue notification: `(slot, queue)` bound to one ioeventfd.
pub(crate) struct QueueNotify {
    pub(crate) slot: Slot,
    pub(crate) queue: u16,
    pub(crate) fd: EventFd,
    address: u64,
}

/// The eight queue-notify ioeventfds, registered with `datamatch` equal to the queue index
/// at each slot's `QueueNotify` register, so an in-range 32-bit notify never exits to userspace.
pub(crate) struct NotifyFds {
    entries: Vec<QueueNotify>,
}

impl NotifyFds {
    /// Creates and registers one ioeventfd per `(slot, queue)` from the bus table.
    pub(crate) fn register(vm: &VmFd, bus: &MmioBus) -> Result<Self, MachineError> {
        let mut registrar = Registrar {
            vm,
            entries: Vec::new(),
        };
        if let Err(error) = bus.register_notify_sources(&mut registrar) {
            registrar.rollback();
            return Err(error);
        }
        Ok(Self {
            entries: registrar.entries,
        })
    }

    pub(crate) fn entries(&self) -> &[QueueNotify] {
        &self.entries
    }

    /// Duplicates every notify descriptor so another thread can kick a queue by `(slot, queue)`.
    pub(crate) fn kicks(&self) -> Result<NotifyKicks, MachineError> {
        let mut kicks = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            kicks.push((
                entry.slot,
                entry.queue,
                entry
                    .fd
                    .try_clone()
                    .map_err(|error| MachineError::io(Phase::Events, &error))?,
            ));
        }
        Ok(NotifyKicks(kicks))
    }

    pub(crate) fn unregister(&mut self, vm: &VmFd) {
        for entry in self.entries.drain(..) {
            let _ignored = vm.unregister_ioevent(
                &entry.fd,
                &IoEventAddress::Mmio(entry.address),
                u32::from(entry.queue),
            );
        }
    }
}

struct Registrar<'a> {
    vm: &'a VmFd,
    entries: Vec<QueueNotify>,
}

impl Registrar<'_> {
    fn rollback(&mut self) {
        for entry in self.entries.drain(..) {
            let _ignored = self.vm.unregister_ioevent(
                &entry.fd,
                &IoEventAddress::Mmio(entry.address),
                u32::from(entry.queue),
            );
        }
    }
}

impl NotifySource for Registrar<'_> {
    type Error = MachineError;

    fn register(&mut self, addr: GuestAddress, slot: Slot, queue: u16) -> Result<(), MachineError> {
        let fd = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Events, &error))?;
        self.vm
            .register_ioevent(&fd, &IoEventAddress::Mmio(addr.raw()), u32::from(queue))
            .map_err(|error| MachineError::os(Phase::Events, error))?;
        self.entries.push(QueueNotify {
            slot,
            queue,
            fd,
            address: addr.raw(),
        });
        Ok(())
    }
}

/// Duplicated notify descriptors for kicking a queue from outside the device thread.
pub(crate) struct NotifyKicks(Vec<(Slot, u16, EventFd)>);

impl NotifyKicks {
    /// Wakes the device thread for `(slot, queue)`; returns false when no such queue exists.
    pub(crate) fn kick(&self, slot: Slot, queue: u16) -> bool {
        self.0
            .iter()
            .find(|(s, q, _)| *s == slot && *q == queue)
            .is_some_and(|(_, _, fd)| fd.write(1).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn irq_lines_map_gsis_to_slot_order_and_reject_foreign_gsis() {
        let mut lines = IrqLines::create().unwrap();
        assert!(lines.signal(FIRST_GSI).is_ok());
        assert!(lines.signal(FIRST_GSI + 4).is_ok());
        assert!(lines.signal(FIRST_GSI + 5).is_err());
        assert!(lines.signal(4).is_err());
        assert!(lines.signal_slot(Slot::Vsock));
        assert_eq!(lines.lines[3].read().unwrap(), 1);
        assert_eq!(lines.lines[0].read().unwrap(), 1);
        assert_eq!(lines.lines[4].read().unwrap(), 1);
    }
}
