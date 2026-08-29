use std::{
    cell::Cell,
    collections::VecDeque,
    fmt,
    io::{self, Write},
};

use kvm_bindings::{KVM_ARM_IRQ_TYPE_SHIFT, KVM_ARM_IRQ_TYPE_SPI};
use kvm_ioctls::VmFd;
use vm_superio::{Serial, Trigger, serial::SerialEvents};

use super::{
    Arm64BootError,
    command::MAX_OUTPUT,
    layout::{CONTROL_UART_BASE, CONTROL_UART_SPI, UART_SIZE},
    protocol::{Decoder, Frame, HEADER_LEN},
};

const UART_REGISTERS: u64 = 8;
const MAX_RESPONSE_FRAMES: usize = MAX_OUTPUT + 2;
const HELLO_PAYLOAD_LEN: usize = 0;
const TERMINAL_PAYLOAD_LEN: usize = 16;
const MAX_RESPONSE_WIRE_BYTES: usize = HEADER_LEN
    + HELLO_PAYLOAD_LEN
    + (HEADER_LEN + 1) * MAX_OUTPUT
    + HEADER_LEN
    + TERMINAL_PAYLOAD_LEN;
const GIC_SPI_BASE: u32 = 32;

pub(crate) struct VmIrqTrigger<'a> {
    vm: &'a VmFd,
}

impl<'a> VmIrqTrigger<'a> {
    pub(crate) const fn new(vm: &'a VmFd) -> Self {
        Self { vm }
    }
}

impl Trigger for VmIrqTrigger<'_> {
    type E = kvm_ioctls::Error;

    fn trigger(&self) -> Result<(), Self::E> {
        let irq =
            (KVM_ARM_IRQ_TYPE_SPI << KVM_ARM_IRQ_TYPE_SHIFT) | (GIC_SPI_BASE + CONTROL_UART_SPI);
        self.vm.set_irq_line(irq, true)?;
        self.vm.set_irq_line(irq, false)
    }
}

#[derive(Default)]
struct RefillEvents(Cell<bool>);

impl RefillEvents {
    fn take(&self) -> bool {
        self.0.replace(false)
    }
}

impl SerialEvents for RefillEvents {
    fn buffer_read(&self) {}
    fn out_byte(&self) {}
    fn tx_lost_byte(&self) {}
    fn in_buffer_empty(&self) {
        self.0.set(true);
    }
}

struct FrameWriter {
    decoder: Decoder,
    completed: VecDeque<Frame>,
    frames: usize,
    wire_bytes: usize,
}

impl FrameWriter {
    fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            completed: VecDeque::new(),
            frames: 0,
            wire_bytes: 0,
        }
    }

    fn take(&mut self) -> Option<Frame> {
        self.completed.pop_front()
    }
}

impl Write for FrameWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for byte in bytes {
            self.wire_bytes = self
                .wire_bytes
                .checked_add(1)
                .ok_or_else(|| io::Error::other("control wire byte count overflow"))?;
            if self.wire_bytes > MAX_RESPONSE_WIRE_BYTES {
                return Err(io::Error::other("control response exceeds wire budget"));
            }
            if let Some(frame) = self
                .decoder
                .push(*byte)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            {
                self.frames = self
                    .frames
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("control frame count overflow"))?;
                if self.frames > MAX_RESPONSE_FRAMES {
                    return Err(io::Error::other("control response exceeds frame budget"));
                }
                self.completed.push_back(frame);
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct ControlUart<T: Trigger> {
    serial: Serial<T, RefillEvents, FrameWriter>,
    request: Vec<u8>,
    request_offset: usize,
    request_started: bool,
}

impl<T> ControlUart<T>
where
    T: Trigger,
    T::E: fmt::Display,
{
    pub(crate) fn new(trigger: T, request: Vec<u8>) -> Self {
        Self {
            serial: Serial::with_events(trigger, RefillEvents::default(), FrameWriter::new()),
            request,
            request_offset: 0,
            request_started: false,
        }
    }

    pub(crate) fn start_request(&mut self) -> Result<(), Arm64BootError> {
        if self.request_started {
            return Err(Arm64BootError::message("control request started twice"));
        }
        self.request_started = true;
        self.refill()
    }

    pub(crate) fn request_drained(&self) -> bool {
        self.request_started
            && self.request_offset == self.request.len()
            && self.serial.fifo_capacity() == 64
    }

    pub(crate) fn read(&mut self, address: u64, data: &mut [u8]) -> Result<(), Arm64BootError> {
        if data.len() != 1 {
            return Err(Arm64BootError::message(
                "control UART MMIO read width must be one byte",
            ));
        }
        let offset = register_offset(address)?;
        data.fill(0);
        if let Some(first) = data.first_mut() {
            *first = self.serial.read(offset);
        }
        if self.serial.events().take() {
            self.refill()?;
        }
        Ok(())
    }

    pub(crate) fn write(
        &mut self,
        address: u64,
        data: &[u8],
    ) -> Result<Option<Frame>, Arm64BootError> {
        let offset = register_offset(address)?;
        if data.len() != 1 {
            return Err(Arm64BootError::message(
                "control UART MMIO write width must be one byte",
            ));
        }
        let value = data[0];
        self.serial
            .write(offset, value)
            .map_err(|error| Arm64BootError::at("emulate control UART write", error))?;
        if offset == 1 && self.request_started {
            self.refill()?;
        }
        Ok(self.serial.writer_mut().take())
    }

    fn refill(&mut self) -> Result<(), Arm64BootError> {
        if !self.request_started
            || self.request_offset == self.request.len()
            || self.serial.state().interrupt_enable & 1 == 0
            || self.serial.fifo_capacity() == 0
        {
            return Ok(());
        }
        let written = self
            .serial
            .enqueue_raw_bytes(&self.request[self.request_offset..])
            .map_err(|error| Arm64BootError::at("fill control UART receive FIFO", error))?;
        self.request_offset = self
            .request_offset
            .checked_add(written)
            .ok_or_else(|| Arm64BootError::message("control request offset overflow"))?;
        Ok(())
    }
}

fn register_offset(address: u64) -> Result<u8, Arm64BootError> {
    let end = CONTROL_UART_BASE
        .checked_add(UART_SIZE)
        .ok_or_else(|| Arm64BootError::message("control UART address overflow"))?;
    if address < CONTROL_UART_BASE || address >= end {
        return Err(Arm64BootError::message(
            "MMIO address is outside control UART",
        ));
    }
    let offset = address - CONTROL_UART_BASE;
    if offset >= UART_REGISTERS {
        return Err(Arm64BootError::message("unsupported control UART register"));
    }
    u8::try_from(offset).map_err(|error| Arm64BootError::at("convert UART register", error))
}

#[cfg(test)]
mod tests;
