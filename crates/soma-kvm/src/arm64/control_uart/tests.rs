use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use vm_superio::Trigger;

use super::*;
use crate::arm64::protocol::{self, Kind};

#[derive(Clone)]
struct TestTrigger(Arc<AtomicUsize>);

impl Trigger for TestTrigger {
    type E = io::Error;

    fn trigger(&self) -> Result<(), Self::E> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn trigger() -> TestTrigger {
    TestTrigger(Arc::new(AtomicUsize::new(0)))
}

#[test]
fn streams_every_byte_value_through_the_64_byte_fifo() {
    let expected: Vec<u8> = (0..=u8::MAX).collect();
    let mut uart = ControlUart::new(trigger(), expected.clone());
    uart.write(CONTROL_UART_BASE + 1, &[1]).unwrap();
    uart.start_request().unwrap();
    let mut actual = Vec::new();
    for _ in 0..expected.len() {
        let mut byte = [0];
        uart.read(CONTROL_UART_BASE, &mut byte).unwrap();
        actual.push(byte[0]);
    }
    assert_eq!(actual, expected);
}

#[test]
fn guest_frames_preserve_binary_payloads() {
    let payload: Vec<u8> = (0..=u8::MAX).collect();
    let encoded = protocol::encode(&Frame {
        kind: Kind::Stdout,
        request_id: 8,
        sequence: 3,
        challenge: [7; 32],
        payload: payload.clone(),
    })
    .unwrap();
    let mut uart = ControlUart::new(trigger(), Vec::new());
    let mut frame = None;
    for byte in encoded {
        frame = uart.write(CONTROL_UART_BASE, &[byte]).unwrap().or(frame);
    }
    assert_eq!(frame.unwrap().payload, payload);
}

#[test]
fn request_waits_for_guest_receive_interrupt_enable() {
    let mut uart = ControlUart::new(trigger(), vec![0xab]);
    uart.start_request().unwrap();
    assert_eq!(uart.serial.fifo_capacity(), 64);
    uart.write(CONTROL_UART_BASE + 1, &[1]).unwrap();
    let mut byte = [0];
    uart.read(CONTROL_UART_BASE, &mut byte).unwrap();
    assert_eq!(byte, [0xab]);
}

#[test]
fn repeated_receive_interrupt_enable_while_fifo_is_full_is_safe() {
    let mut uart = ControlUart::new(trigger(), vec![0xab; 65]);
    uart.write(CONTROL_UART_BASE + 1, &[1]).unwrap();
    uart.start_request().unwrap();
    assert_eq!(uart.serial.fifo_capacity(), 0);
    uart.write(CONTROL_UART_BASE + 1, &[1]).unwrap();
    assert_eq!(uart.serial.fifo_capacity(), 0);
}

#[test]
fn request_is_not_drained_after_only_the_first_fifo() {
    let mut uart = ControlUart::new(trigger(), vec![0xab; 129]);
    uart.write(CONTROL_UART_BASE + 1, &[1]).unwrap();
    uart.start_request().unwrap();
    for _ in 0..64 {
        uart.read(CONTROL_UART_BASE, &mut [0]).unwrap();
    }
    assert!(!uart.request_drained());
}

#[test]
fn rejects_non_byte_accesses_and_register_boundary_overruns() {
    let mut uart = ControlUart::new(trigger(), Vec::new());
    for width in [0, 2, 4, 8] {
        assert!(uart.write(CONTROL_UART_BASE, &vec![0; width]).is_err());
        assert!(uart.read(CONTROL_UART_BASE, &mut vec![0; width]).is_err());
    }
    assert!(uart.write(CONTROL_UART_BASE - 1, &[0]).is_err());
    assert!(
        uart.write(CONTROL_UART_BASE + UART_REGISTERS, &[0])
            .is_err()
    );
}

#[test]
fn exact_worst_case_one_byte_frames_fit_the_wire_budget() {
    let mut writer = FrameWriter::new();
    let identity = |kind, sequence, payload| Frame {
        kind,
        request_id: 8,
        sequence,
        challenge: [7; 32],
        payload,
    };
    writer
        .write_all(
            &protocol::encode(&Frame {
                kind: Kind::Hello,
                request_id: 0,
                sequence: 0,
                challenge: [0; 32],
                payload: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
    for sequence in 0..u32::try_from(MAX_OUTPUT).unwrap() {
        writer
            .write_all(&protocol::encode(&identity(Kind::Stdout, sequence, vec![b'x'])).unwrap())
            .unwrap();
    }
    writer
        .write_all(
            &protocol::encode(&identity(
                Kind::Terminal,
                u32::try_from(MAX_OUTPUT).unwrap(),
                vec![0; TERMINAL_PAYLOAD_LEN],
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(writer.wire_bytes, MAX_RESPONSE_WIRE_BYTES);
    assert!(writer.write_all(&[0]).is_err());
}
