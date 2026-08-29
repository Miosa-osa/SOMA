//! Checked dispatch of `x86` port I/O exits to the few ports the machine answers.
//!
//! The serial model owns `0x3f8..0x400`. The keyboard-controller command port `0x64` is watched
//! only for the `0xfe` CPU-reset pulse that `reboot=k` issues, which the machine treats as an
//! orderly reset request. Every other port reads as a floating bus and ignores writes, and every
//! access class is counted so the evidence can show exactly what the guest touched.

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

/// Bounded counts of every port-access class, by device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusCounters {
    pub serial_in: u64,
    pub serial_out: u64,
    pub i8042_in: u64,
    pub i8042_out: u64,
    pub other_in: u64,
    pub other_out: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> PortBus {
        PortBus::new(Serial::new(None))
    }

    #[test]
    fn routes_serial_bytes_and_counts_them() {
        let mut bus = bus();
        assert_eq!(bus.io_out(SERIAL_BASE, b"S").unwrap(), PortEvent::Continue);
        assert_eq!(bus.io_out(SERIAL_BASE, b"OM").unwrap(), PortEvent::Continue);
        let mut data = [0_u8; 1];
        bus.io_in(SERIAL_BASE + 5, &mut data);
        assert_eq!(data, [0x60]);
        let mut wide = [0_u8; 2];
        bus.io_in(SERIAL_BASE + 5, &mut wide);
        assert_eq!(wide, [0xff, 0xff]);
        assert_eq!(bus.serial().output(), b"S");
        assert_eq!(bus.counters().serial_out, 2);
        assert_eq!(bus.counters().serial_in, 2);
        assert_eq!(bus.serial_counters().thr_writes, 1);
        assert_eq!(bus.into_serial().into_output(), b"S");
    }

    #[test]
    fn keyboard_controller_reset_pulse_is_an_orderly_reset() {
        let mut bus = bus();
        let mut status = [0xaa_u8; 1];
        bus.io_in(I8042_COMMAND_PORT, &mut status);
        assert_eq!(status, [0]);
        assert_eq!(
            bus.io_out(I8042_COMMAND_PORT, &[0xd1]).unwrap(),
            PortEvent::Continue
        );
        assert_eq!(
            bus.io_out(I8042_DATA_PORT, &[0xfe]).unwrap(),
            PortEvent::Continue
        );
        assert_eq!(
            bus.io_out(I8042_COMMAND_PORT, &[0xfe]).unwrap(),
            PortEvent::Reset
        );
        assert_eq!(bus.counters().i8042_in, 1);
        assert_eq!(bus.counters().i8042_out, 3);
    }

    #[test]
    fn other_ports_float_and_are_counted() {
        let mut bus = bus();
        let mut data = [0_u8; 4];
        bus.io_in(0xcf8, &mut data);
        assert_eq!(data, [0xff; 4]);
        assert_eq!(bus.io_out(0x80, &[0]).unwrap(), PortEvent::Continue);
        assert_eq!(bus.counters().other_in, 1);
        assert_eq!(bus.counters().other_out, 1);
        assert_eq!(bus.counters().serial_in, 0);
    }
}
