#[cfg(test)]
use super::ARM64_BOOT_SENTINEL;
use super::layout::{UART_BASE, UART_SIZE};

const MAX_CONSOLE_BYTES: usize = 1024 * 1024;
const DATA: usize = 0;
const INTERRUPT_ID: usize = 2;
const LINE_CONTROL: usize = 3;
const LINE_STATUS: usize = 5;
const DLAB: u8 = 1 << 7;
const TX_EMPTY: u8 = (1 << 5) | (1 << 6);

pub(crate) struct Uart {
    registers: [u8; 8],
    console: Vec<u8>,
    expected_sentinel: &'static [u8],
    capture: bool,
}

impl Uart {
    pub(crate) fn new(expected_sentinel: &'static [u8]) -> Self {
        Self {
            registers: [0; 8],
            console: Vec::new(),
            expected_sentinel,
            capture: true,
        }
    }

    pub(crate) fn read(&self, address: u64, data: &mut [u8]) -> Result<(), &'static str> {
        data.fill(0);
        let offset = Self::offset(address)?;
        let value = match offset {
            INTERRUPT_ID => 1,
            LINE_STATUS => TX_EMPTY,
            0..=7 => self.registers[offset],
            _ => 0,
        };
        if let Some(first) = data.first_mut() {
            *first = value;
        }
        Ok(())
    }

    pub(crate) fn write(&mut self, address: u64, data: &[u8]) -> Result<bool, &'static str> {
        let offset = Self::offset(address)?;
        let Some(&value) = data.first() else {
            return Ok(false);
        };
        if offset == DATA && self.registers[LINE_CONTROL] & DLAB == 0 {
            if self.capture {
                if self.console.len() == MAX_CONSOLE_BYTES {
                    return Err("serial console exceeded one MiB before the sentinel");
                }
                self.console.push(value);
            }
        } else if offset <= 7 {
            self.registers[offset] = value;
        }
        Ok(self.console.ends_with(self.expected_sentinel))
    }

    pub(crate) fn into_console(self) -> Vec<u8> {
        self.console
    }

    pub(crate) fn console(&self) -> &[u8] {
        &self.console
    }

    pub(crate) fn stop_capture(&mut self) {
        self.console.clear();
        self.capture = false;
    }

    fn offset(address: u64) -> Result<usize, &'static str> {
        let offset = address
            .checked_sub(UART_BASE)
            .ok_or("MMIO access is below the serial device")?;
        if offset >= UART_SIZE {
            return Err("MMIO access is outside the serial device");
        }
        usize::try_from(offset).map_err(|_| "serial MMIO offset does not fit in usize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_only_transmit_writes_and_reports_sentinel() {
        let mut uart = Uart::new(ARM64_BOOT_SENTINEL.as_bytes());
        for (index, &byte) in ARM64_BOOT_SENTINEL.as_bytes().iter().enumerate() {
            let reached = uart.write(UART_BASE, &[byte]).unwrap();
            assert_eq!(reached, index + 1 == ARM64_BOOT_SENTINEL.len());
        }
        assert_eq!(uart.into_console(), ARM64_BOOT_SENTINEL.as_bytes());
    }

    #[test]
    fn reports_transmitter_ready() {
        let uart = Uart::new(ARM64_BOOT_SENTINEL.as_bytes());
        let mut value = [0_u8; 4];
        uart.read(UART_BASE + 5, &mut value).unwrap();
        assert_eq!(value, [TX_EMPTY, 0, 0, 0]);
    }

    #[test]
    fn stops_retaining_diagnostic_bytes_after_the_handshake() {
        let mut uart = Uart::new(ARM64_BOOT_SENTINEL.as_bytes());
        uart.write(UART_BASE, b"b").unwrap();
        uart.stop_capture();
        for _ in 0..=MAX_CONSOLE_BYTES {
            assert!(!uart.write(UART_BASE, b"x").unwrap());
        }
        assert!(uart.console().is_empty());
        let mut value = [0_u8];
        uart.read(UART_BASE, &mut value).unwrap();
        assert_eq!(value, [0]);
    }
}
