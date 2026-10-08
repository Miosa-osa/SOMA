//! Unit tests for the port bus and the counter record it keeps.
//!
//! Split from `ports.rs` so the bus stays one screen and the evidence rules stay readable.

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
    assert_eq!(bus.counters().unmodelled_ports(), [0xcf8, 0x80]);
}

#[test]
fn unmodelled_ports_are_named_once_and_the_list_is_bounded() {
    let mut bus = bus();
    for _ in 0..3 {
        bus.io_out(0x21, &[0xff]).unwrap();
    }
    assert_eq!(bus.counters().unmodelled_ports(), [0x21]);
    for port in [0x22_u16, 0x23, 0x24, 0x25, 0x26] {
        bus.io_out(port, &[0]).unwrap();
    }
    assert_eq!(bus.counters().other_out, 8);
    assert_eq!(
        bus.counters().unmodelled_ports().len(),
        OTHER_PORTS_REMEMBERED
    );
    assert_eq!(bus.counters().unmodelled_ports(), [0x21, 0x22, 0x23, 0x24]);
}
