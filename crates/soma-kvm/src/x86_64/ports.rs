//! Checked dispatch of `x86` port I/O exits to the few ports the machine answers.
//!
//! The serial model owns `0x3f8..0x400`. The keyboard-controller command port `0x64` is watched
//! only for the `0xfe` CPU-reset pulse that `reboot=k` issues, which the machine treats as an
//! orderly reset request. Every other port reads as a floating bus and ignores writes, and every
//! access class is counted, and the unmodelled ports themselves are named, so the evidence can
//! show exactly what the guest touched.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{
    error::MachineError,
    serial::{SERIAL_BASE, SERIAL_PORTS, Serial, SerialCounters},
};

const I8042_DATA_PORT: u16 = 0x60;
const I8042_COMMAND_PORT: u16 = 0x64;
const I8042_CPU_RESET: u8 = 0xfe;
const FLOATING_BUS: u8 = 0xff;

/// What the bus asks the run loop to do after a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortEvent {
    Continue,
    Reset,
}

/// How many distinct unmodelled ports the evidence remembers by name.
///
/// Accesses past the bound are still counted, so a guest cannot grow the evidence without end.
pub const OTHER_PORTS_REMEMBERED: usize = 4;

/// Bounded counts of every port-access class, by device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusCounters {
    pub serial_in: u64,
    pub serial_out: u64,
    pub i8042_in: u64,
    pub i8042_out: u64,
    pub other_in: u64,
    pub other_out: u64,
    /// The unmodelled ports touched, in first-touch order; the tail is unused. Written only
    /// through [`BusCounters::note_unmodelled_port`] so the count and the names stay in step.
    other_ports: [u16; OTHER_PORTS_REMEMBERED],
    /// How many entries of `other_ports` are in use.
    other_ports_used: u8,
}

impl BusCounters {
    /// Names a port the machine does not model, once, in first-touch order.
    ///
    /// Naming the port is what lets a reader tell a guest walking the legacy devices the machine
    /// declares absent from one probing for a device that was never declared; a bare count
    /// cannot. Past [`OTHER_PORTS_REMEMBERED`] distinct ports the accesses keep counting and the
    /// names stop growing.
    fn note_unmodelled_port(&mut self, port: u16) {
        let used = usize::from(self.other_ports_used).min(OTHER_PORTS_REMEMBERED);
        if self.other_ports[..used].contains(&port) {
            return;
        }
        if let Some(slot) = self.other_ports.get_mut(used) {
            *slot = port;
            self.other_ports_used = self.other_ports_used.saturating_add(1);
        }
    }

    /// The distinct ports the guest touched that the machine does not model, in first-touch order.
    #[must_use]
    pub fn unmodelled_ports(&self) -> &[u16] {
        let used = usize::from(self.other_ports_used).min(OTHER_PORTS_REMEMBERED);
        &self.other_ports[..used]
    }
}

fn bump(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

/// The machine's complete port-I/O surface.
pub(crate) struct PortBus {
    serial: Serial,
    counters: BusCounters,
}

impl PortBus {
    pub(crate) fn new(serial: Serial) -> Self {
        Self {
            serial,
            counters: BusCounters::default(),
        }
    }

    pub(crate) const fn serial(&self) -> &Serial {
        &self.serial
    }

    pub(crate) fn into_serial(self) -> Serial {
        self.serial
    }

    pub(crate) const fn counters(&self) -> BusCounters {
        self.counters
    }

    pub(crate) const fn serial_counters(&self) -> SerialCounters {
        self.serial.counters()
    }

    /// Fills `data` for an `in` from `port`; multi-byte accesses never reach a device.
    pub(crate) fn io_in(&mut self, port: u16, data: &mut [u8]) {
        match Self::classify(port) {
            Target::Serial(offset) if data.len() == 1 => {
                bump(&mut self.counters.serial_in);
                data[0] = self.serial.read(offset);
            }
            Target::Serial(_) => {
                bump(&mut self.counters.serial_in);
                data.fill(FLOATING_BUS);
            }
            Target::I8042 => {
                bump(&mut self.counters.i8042_in);
                // Input and output buffers empty: the reset sequence proceeds without waiting.
                data.fill(0);
            }
            Target::Other => {
                bump(&mut self.counters.other_in);
                self.counters.note_unmodelled_port(port);
                data.fill(FLOATING_BUS);
            }
        }
    }

    /// Applies an `out` of `data` to `port`.
    pub(crate) fn io_out(&mut self, port: u16, data: &[u8]) -> Result<PortEvent, MachineError> {
        match Self::classify(port) {
            Target::Serial(offset) => {
                bump(&mut self.counters.serial_out);
                if let [byte] = data {
                    self.serial.write(offset, *byte)?;
                }
                Ok(PortEvent::Continue)
            }
            Target::I8042 => {
                bump(&mut self.counters.i8042_out);
                if port == I8042_COMMAND_PORT && data == [I8042_CPU_RESET] {
                    Ok(PortEvent::Reset)
                } else {
                    Ok(PortEvent::Continue)
                }
            }
            Target::Other => {
                bump(&mut self.counters.other_out);
                self.counters.note_unmodelled_port(port);
                Ok(PortEvent::Continue)
            }
        }
    }

    fn classify(port: u16) -> Target {
        match port.checked_sub(SERIAL_BASE) {
            Some(offset) if offset < SERIAL_PORTS => Target::Serial(offset),
            _ if port == I8042_DATA_PORT || port == I8042_COMMAND_PORT => Target::I8042,
            _ => Target::Other,
        }
    }
}

enum Target {
    Serial(u16),
    I8042,
    Other,
}

/// The port bus shared by every vCPU thread of one machine.
///
/// A machine has one console, one 16550 model, and one set of port counters whatever its vCPU
/// count, so every run loop shares this one bus behind a mutex instead of owning a private copy.
/// A private copy would split the captured diagnostic console and undercount the evidence by
/// whichever processor happened to touch a port, which is exactly the kind of silently weaker
/// evidence the machine refuses elsewhere. Port access is rare - the console is the only device
/// on it - so the lock is never contended in practice.
#[derive(Clone)]
pub(crate) struct PortBusHandle(Arc<Mutex<PortBus>>);

impl PortBusHandle {
    pub(crate) fn new(bus: PortBus) -> Self {
        Self(Arc::new(Mutex::new(bus)))
    }

    /// Locks the bus, recovering a poisoned lock because every state it guards is bounded
    /// counters and a byte buffer, and the guest is about to be torn down anyway.
    fn lock(&self) -> MutexGuard<'_, PortBus> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn io_in(&self, port: u16, data: &mut [u8]) {
        self.lock().io_in(port, data);
    }

    pub(crate) fn io_out(&self, port: u16, data: &[u8]) -> Result<PortEvent, MachineError> {
        self.lock().io_out(port, data)
    }

    /// Runs `read` against the console while the bus is locked, so nothing copies the whole
    /// captured console on every port write.
    pub(crate) fn with_serial<R>(&self, read: impl FnOnce(&Serial) -> R) -> R {
        read(self.lock().serial())
    }

    /// Takes the bus back once every thread that held a clone has stopped.
    ///
    /// Returns `None` while another owner remains, which is a caller that asked before its
    /// workers were joined; that is a programming error rather than a guest condition.
    pub(crate) fn into_inner(self) -> Option<PortBus> {
        Arc::try_unwrap(self.0)
            .ok()
            .map(|bus| bus.into_inner().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests;
