//! Dispatch of `KVM_EXIT_MMIO` to the shared virtio bus on the vCPU thread.
//!
//! Only the five fixed transport pages are answerable. An access outside them is a typed
//! fatal exit because nothing else is mapped there; a transport-level violation inside them
//! is counted and observed by the guest as a zero read or a dropped write, exactly as the bus
//! documents, so a hostile driver cannot stop the machine by poking its own device.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::{
    devices::SharedBus,
    error::{MachineError, MachineErrorKind, Phase},
    events::NotifyKicks,
    memory::SharedRam,
};
use crate::virtio::{AccessWidth, BusViolation};

/// Bounded counts of what the vCPU thread dispatched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MmioCounters {
    pub reads: u64,
    pub writes: u64,
    pub transport_violations: u64,
    pub notify_exits: u64,
}

fn bump(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

/// The vCPU thread's handle on the bus, guest memory, and queue kicks.
pub(crate) struct MmioDispatch {
    shared: Arc<SharedBus>,
    memory: SharedRam,
    kicks: NotifyKicks,
    finished: Arc<AtomicBool>,
    counters: MmioCounters,
}

impl MmioDispatch {
    pub(crate) fn new(
        shared: Arc<SharedBus>,
        memory: SharedRam,
        kicks: NotifyKicks,
        finished: Arc<AtomicBool>,
    ) -> Self {
        Self {
            shared,
            memory,
            kicks,
            finished,
            counters: MmioCounters::default(),
        }
    }

    pub(crate) const fn counters(&self) -> MmioCounters {
        self.counters
    }

    /// Fills `data` for a guest read at `address`.
    pub(crate) fn read(&mut self, address: u64, data: &mut [u8]) -> Result<(), MachineError> {
        bump(&mut self.counters.reads);
        let width = width_of(data.len())?;
        let value = {
            let mut bus = self.shared.lock();
            match bus.dispatch_read(address, width) {
                Ok(value) => value,
                Err(BusViolation::UnmappedAddress { gpa }) => return Err(unmapped(gpa)),
                Err(BusViolation::Transport { .. }) => {
                    bump(&mut self.counters.transport_violations);
                    0
                }
            }
        };
        data.copy_from_slice(&value.to_le_bytes()[..data.len()]);
        Ok(())
    }

    /// Applies a guest write of `data` at `address`.
    pub(crate) fn write(&mut self, address: u64, data: &[u8]) -> Result<(), MachineError> {
        bump(&mut self.counters.writes);
        let width = width_of(data.len())?;
        let mut raw = [0_u8; 8];
        raw[..data.len()].copy_from_slice(data);
        let value = u64::from_le_bytes(raw);
        let notify = {
            let mut bus = self.shared.lock();
            match bus.dispatch_write(address, width, value, &self.memory) {
                Ok(event) => event.notify(),
                Err(BusViolation::UnmappedAddress { gpa }) => return Err(unmapped(gpa)),
                Err(BusViolation::Transport { .. }) => {
                    bump(&mut self.counters.transport_violations);
                    None
                }
            }
        };
        if let Some((slot, queue)) = notify {
            // An in-range notify is normally absorbed by its ioeventfd; a write that still
            // exited is forwarded to the device thread rather than serviced under the vCPU.
            bump(&mut self.counters.notify_exits);
            self.kicks.kick(slot, queue);
        }
        Ok(())
    }

    /// Marks the vCPU as stopped so control-channel waiters fail fast instead of waiting out
    /// their deadlines against a guest that no longer runs.
    pub(crate) fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        self.shared.notify_all();
    }
}

fn width_of(bytes: usize) -> Result<AccessWidth, MachineError> {
    match bytes {
        1 => Ok(AccessWidth::U8),
        2 => Ok(AccessWidth::U16),
        4 => Ok(AccessWidth::U32),
        8 => Ok(AccessWidth::U64),
        _ => Err(MachineError::new(
            Phase::Run,
            MachineErrorKind::MmioWidth { bytes },
        )),
    }
}

fn unmapped(address: u64) -> MachineError {
    MachineError::new(Phase::Run, MachineErrorKind::UnmappedMmio { address })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_the_exit_payload_length() {
        assert_eq!(width_of(1).unwrap(), AccessWidth::U8);
        assert_eq!(width_of(2).unwrap(), AccessWidth::U16);
        assert_eq!(width_of(4).unwrap(), AccessWidth::U32);
        assert_eq!(width_of(8).unwrap(), AccessWidth::U64);
        for bytes in [0, 3, 5, 16] {
            let error = width_of(bytes).unwrap_err();
            assert_eq!(error.kind(), &MachineErrorKind::MmioWidth { bytes });
            assert_eq!(error.phase(), Phase::Run);
        }
    }
}
