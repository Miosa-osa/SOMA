use vmm_sys_util::eventfd::EventFd;

use super::*;

fn model() -> Serial {
    Serial::new(None)
}

#[test]
fn transmit_writes_are_captured_in_order_and_bounded() {
    let mut serial = model();
    for byte in b"SOMA" {
        serial.write(0, *byte).unwrap();
    }
    assert_eq!(serial.output(), b"SOMA");
    assert_eq!(serial.counters().thr_writes, 4);
    for _ in 4..SERIAL_CAPTURE_LIMIT {
        serial.write(0, b'x').unwrap();
    }
    let error = serial.write(0, b'y').unwrap_err();
    assert_eq!(error.phase(), Phase::Run);
    assert_eq!(serial.output().len(), SERIAL_CAPTURE_LIMIT);
}

#[test]
fn line_status_always_reports_an_empty_transmitter() {
    let mut serial = model();
    assert_eq!(serial.read(5), LSR_THRE_TEMT);
    assert_eq!(serial.counters().lsr_reads, 1);
    assert_eq!(serial.read(0), 0);
}

#[test]
fn divisor_latch_selects_baud_registers() {
    let mut serial = model();
    serial.write(3, LCR_DLAB).unwrap();
    serial.write(0, 0x0c).unwrap();
    serial.write(1, 0x00).unwrap();
    assert_eq!(serial.read(0), 0x0c);
    assert_eq!(serial.read(1), 0x00);
    assert!(serial.output().is_empty());
    serial.write(3, 0x03).unwrap();
    assert_eq!(serial.read(3), 0x03);
    serial.write(0, b'!').unwrap();
    assert_eq!(serial.output(), b"!");
}

#[test]
fn scratch_and_modem_control_round_trip_for_autoconfig() {
    let mut serial = model();
    serial.write(7, 0xa5).unwrap();
    assert_eq!(serial.read(7), 0xa5);
    serial.write(7, 0x5a).unwrap();
    assert_eq!(serial.read(7), 0x5a);
    serial.write(4, MCR_LOOP | 0x0a).unwrap();
    assert_eq!(serial.read(4), MCR_LOOP | 0x0a);
    assert_eq!(serial.read(6) & 0xf0, 0x90);
    serial.write(4, 0).unwrap();
    assert_eq!(serial.read(6), MSR_IDLE);
}

#[test]
fn fifo_enable_is_reflected_in_iir() {
    let mut serial = model();
    assert_eq!(serial.read(2), IIR_NO_INTERRUPT);
    serial.write(2, FCR_ENABLE_FIFO).unwrap();
    assert_eq!(serial.read(2), IIR_FIFO_ENABLED | IIR_NO_INTERRUPT);
    assert_eq!(serial.counters().iir_reads, 2);
}

#[test]
fn ier_is_masked_and_thri_is_acknowledged_by_reading_iir() {
    let line = EventFd::new(libc::EFD_NONBLOCK).unwrap();
    let observer = line.try_clone().unwrap();
    let mut serial = Serial::new(Some(line));
    serial.write(1, 0x40).unwrap();
    assert_eq!(serial.read(1), 0);
    assert_eq!(serial.read(2), IIR_NO_INTERRUPT);
    serial.write(1, IER_THRI).unwrap();
    assert_eq!(serial.read(1), IER_THRI);
    assert_eq!(observer.read().unwrap(), 1);
    assert_eq!(serial.read(2), IIR_THRI);
    assert_eq!(serial.read(2), IIR_NO_INTERRUPT);
    serial.write(0, b'a').unwrap();
    assert_eq!(observer.read().unwrap(), 1);
    assert_eq!(serial.read(2), IIR_THRI);
    serial.write(1, 0).unwrap();
    assert_eq!(serial.read(2), IIR_NO_INTERRUPT);
    assert_eq!(serial.counters().interrupts_raised, 2);
    assert_eq!(serial.counters().ier_writes, 3);
}

#[test]
fn unknown_offsets_are_counted_and_harmless() {
    let mut serial = model();
    serial.write(9, 1).unwrap();
    assert_eq!(serial.read(9), 0xff);
    assert_eq!(serial.counters().other_writes, 1);
    assert_eq!(serial.counters().other_reads, 1);
    assert_eq!(serial.into_output(), Vec::<u8>::new());
}
