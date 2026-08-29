//! A bounded output-only 16550 model on the diagnostic port `0x3f8`.
//!
//! The model exists so the Linux 8250 driver detects `ttyS0`, treats the transmitter as always
//! empty, and never blocks on a receive path that does not exist. Guest writes to the transmit
//! holding register append to a bounded capture buffer; overflow is a typed failure. When a
//! transmit-interrupt line is attached, enabling `THRI` raises it so tty writes do not stall.

#[cfg(test)]
mod tests;

use vmm_sys_util::eventfd::EventFd;

use super::error::{MachineError, Phase};

/// The first of the eight `x86` I/O ports owned by the model.
pub(crate) const SERIAL_BASE: u16 = 0x3f8;
/// Number of consecutive ports the model claims.
pub(crate) const SERIAL_PORTS: u16 = 8;
/// The legacy GSI for `COM1`.
pub(crate) const SERIAL_GSI: u32 = 4;
/// Upper bound on captured serial bytes before the proof fails closed.
pub(crate) const SERIAL_CAPTURE_LIMIT: usize = 64 * 1024;

const IER_THRI: u8 = 0x02;
const IER_MASK: u8 = 0x0f;
const IIR_NO_INTERRUPT: u8 = 0x01;
const IIR_THRI: u8 = 0x02;
const IIR_FIFO_ENABLED: u8 = 0xc0;
const FCR_ENABLE_FIFO: u8 = 0x01;
const LCR_DLAB: u8 = 0x80;
const MCR_MASK: u8 = 0x1f;
const MCR_LOOP: u8 = 0x10;
/// Transmitter holding register empty and transmitter empty.
const LSR_THRE_TEMT: u8 = 0x60;
/// Clear to send, data set ready, and carrier detect asserted.
const MSR_IDLE: u8 = 0xb0;

/// Bounded counts of every port-access class the model saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SerialCounters {
    pub thr_writes: u64,
    pub ier_writes: u64,
    pub lsr_reads: u64,
    pub iir_reads: u64,
    pub other_reads: u64,
    pub other_writes: u64,
    pub interrupts_raised: u64,
}

impl SerialCounters {
    fn bump(counter: &mut u64) {
        *counter = counter.saturating_add(1);
    }
}

/// The 16550 register file and capture buffer.
pub(crate) struct Serial {
    ier: u8,
    lcr: u8,
    mcr: u8,
    scratch: u8,
    divisor: [u8; 2],
    fifo_enabled: bool,
    thri_pending: bool,
    output: Vec<u8>,
    counters: SerialCounters,
    interrupt: Option<EventFd>,
}

impl Serial {
    /// Creates a model that raises `interrupt` when the guest enables transmit interrupts.
    pub(crate) fn new(interrupt: Option<EventFd>) -> Self {
        Self {
            ier: 0,
            lcr: 0,
            mcr: 0,
            scratch: 0,
            divisor: [0; 2],
            fifo_enabled: false,
            thri_pending: false,
            output: Vec::new(),
            counters: SerialCounters::default(),
            interrupt,
        }
    }

    pub(crate) fn output(&self) -> &[u8] {
        &self.output
    }

    pub(crate) fn into_output(self) -> Vec<u8> {
        self.output
    }

    pub(crate) const fn counters(&self) -> SerialCounters {
        self.counters
    }

    /// Handles a one-byte guest write to `SERIAL_BASE + offset`.
    pub(crate) fn write(&mut self, offset: u16, value: u8) -> Result<(), MachineError> {
        match (offset, self.dlab()) {
            (0, true) => self.divisor[0] = value,
            (1, true) => self.divisor[1] = value,
            (0, false) => {
                SerialCounters::bump(&mut self.counters.thr_writes);
                self.capture(value)?;
                if self.ier & IER_THRI != 0 {
                    self.raise();
                }
            }
            (1, false) => {
                SerialCounters::bump(&mut self.counters.ier_writes);
                self.ier = value & IER_MASK;
                if self.ier & IER_THRI != 0 {
                    self.raise();
                } else {
                    self.thri_pending = false;
                }
            }
            (2, _) => self.fifo_enabled = value & FCR_ENABLE_FIFO != 0,
            (3, _) => self.lcr = value,
            (4, _) => self.mcr = value & MCR_MASK,
            (7, _) => self.scratch = value,
            _ => SerialCounters::bump(&mut self.counters.other_writes),
        }
        Ok(())
    }

    /// Handles a one-byte guest read from `SERIAL_BASE + offset`.
    pub(crate) fn read(&mut self, offset: u16) -> u8 {
        match (offset, self.dlab()) {
            (0, true) => self.divisor[0],
            (1, true) => self.divisor[1],
            (0, false) => {
                SerialCounters::bump(&mut self.counters.other_reads);
                0
            }
            (1, false) => self.ier,
            (2, _) => {
                SerialCounters::bump(&mut self.counters.iir_reads);
                self.interrupt_identification()
            }
            (3, _) => self.lcr,
            (4, _) => self.mcr,
            (5, _) => {
                SerialCounters::bump(&mut self.counters.lsr_reads);
                LSR_THRE_TEMT
            }
            (6, _) => self.modem_status(),
            (7, _) => self.scratch,
            _ => {
                SerialCounters::bump(&mut self.counters.other_reads);
                0xff
            }
        }
    }

    const fn dlab(&self) -> bool {
        self.lcr & LCR_DLAB != 0
    }

    /// Reading IIR acknowledges a pending transmit interrupt, as on real hardware.
    fn interrupt_identification(&mut self) -> u8 {
        let fifo = if self.fifo_enabled {
            IIR_FIFO_ENABLED
        } else {
            0
        };
        if self.thri_pending {
            self.thri_pending = false;
            fifo | IIR_THRI
        } else {
            fifo | IIR_NO_INTERRUPT
        }
    }

    /// In loopback the modem inputs mirror the modem-control outputs; otherwise the line is idle.
    const fn modem_status(&self) -> u8 {
        if self.mcr & MCR_LOOP == 0 {
            return MSR_IDLE;
        }
        let mut status = 0;
        if self.mcr & 0x02 != 0 {
            status |= 0x10;
        }
        if self.mcr & 0x01 != 0 {
            status |= 0x20;
        }
        if self.mcr & 0x04 != 0 {
            status |= 0x40;
        }
        if self.mcr & 0x08 != 0 {
            status |= 0x80;
        }
        status
    }

    fn capture(&mut self, byte: u8) -> Result<(), MachineError> {
        if self.output.len() >= SERIAL_CAPTURE_LIMIT {
            return Err(MachineError::invalid(
                Phase::Run,
                "serial capture exceeded 64 KiB before the guest stopped",
            ));
        }
        self.output.push(byte);
        Ok(())
    }

    fn raise(&mut self) {
        self.thri_pending = true;
        if let Some(line) = &self.interrupt {
            // A failed eventfd write only delays the guest's tty flush; it never corrupts state.
            if line.write(1).is_ok() {
                SerialCounters::bump(&mut self.counters.interrupts_raised);
            }
        }
    }
}
