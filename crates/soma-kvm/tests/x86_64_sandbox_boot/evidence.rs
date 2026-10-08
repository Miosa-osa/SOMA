//! The evidence table one live sandbox run prints, and what its counters must say.

use std::{fs, path::Path};

use soma_kvm::x86_64::{BusCounters, ExitReason, SandboxEvidence};

/// Prints the timeline, phases, counters, and console tail; retains the console log.
pub fn report(label: &str, evidence: &SandboxEvidence, log: &Path) {
    fs::write(log, &evidence.serial).unwrap();
    let text = String::from_utf8_lossy(&evidence.serial);
    let lines: Vec<&str> = text.lines().collect();
    eprintln!(
        "[{label}] serial log ({} bytes, {} lines) retained at {}",
        evidence.serial.len(),
        lines.len(),
        log.display()
    );
    for line in lines.iter().rev().take(16).rev() {
        eprintln!("  | {line}");
    }
    eprintln!("[{label}] COLD timeline (ns since sandbox creation began; delta from previous):");
    let mut previous = 0;
    for mark in &evidence.timeline {
        eprintln!(
            "  {:<20} {:>14} {:>+14}",
            format!("{:?}", mark.milestone),
            mark.elapsed_ns,
            i128::from(mark.elapsed_ns) - i128::from(previous)
        );
        previous = mark.elapsed_ns;
    }
    for timing in &evidence.phases {
        eprintln!(
            "  phase={:?} elapsed_ns={}",
            timing.phase(),
            timing.elapsed_ns()
        );
    }
    eprintln!(
        "[{label}] cmdline={:?} entry={:#x} initramfs={:?} exit={:?} launch_page_retired={}",
        evidence.cmdline,
        evidence.entry,
        evidence.initramfs,
        evidence.exit,
        evidence.launch_page_retired
    );
    eprintln!(
        "[{label}] bus={:?} uart={:?} mmio={:?}",
        evidence.bus, evidence.uart, evidence.mmio
    );
    eprintln!("[{label}] devices={:?}", evidence.devices);
    // Each processor's own outcome, and the exits the whole machine made, because an application
    // processor that stopped or never started leaves the bootstrap processor waiting and the
    // machine's single result reads as a plain timeout.
    eprintln!("[{label}] vcpus={:?}", evidence.vcpus);
    eprintln!(
        "[{label}] exits port_in={} port_out={} mmio={} halt={} interrupted={} other={} sampled={} inside_ns={} outside_ns={}",
        evidence.exits.of(ExitReason::PortIn),
        evidence.exits.of(ExitReason::PortOut),
        evidence.exits.of(ExitReason::Mmio),
        evidence.exits.of(ExitReason::Halt),
        evidence.exits.of(ExitReason::Interrupted),
        evidence.exits.of(ExitReason::Other),
        evidence.exits.sampled,
        evidence.exits.inside_ns,
        evidence.exits.outside_ns,
    );
}

/// The ISA-era ports a guest may reach once its machine hands interrupts to the I/O APIC.
///
/// A single-vCPU machine boots with `noapic`, so its guest never needs any of these. A
/// multi-vCPU machine boots without it, and on the way to symmetric I/O mode the kernel walks
/// the legacy devices the MP table told it about: it masks the 8259 pair, it may program the
/// 8254 the timer falls back to, and the `outb_p`-style accessors it uses for those writes
/// strobe the delay port. This machine models none of them, so every such access floats on the
/// bus and is counted there, and this list is what one may name.
const LEGACY_PORTS: [u16; 12] = [
    0x20, 0x21, // master 8259A command and data
    0x40, 0x43, // 8254 counter 0 and mode register
    0x61, // speaker gate and NMI status latch
    0x70, 0x71, // CMOS/RTC address and data, the pair the kernel keeps its clock in
    0x80, // the delay port the `outb_p` and `inb_p` accessors write
    0xa0, 0xa1, // slave 8259A command and data
    0x4d0, 0x4d1, // PCI interrupt-router edge and level control registers
];

/// Asserts that the only unmodelled ports the guest reached are the ones its machine explains.
///
/// A guest under a single-vCPU machine has no reason to touch an unmodelled port, so any access
/// is a surprise. A guest under a multi-vCPU machine masks the legacy interrupt controllers,
/// which is expected. Either way a port outside the expected set is a guest looking for a device
/// the machine has not declared, and the failure names the ports so the set can be checked
/// against reality instead of widened by guess.
pub fn assert_ports_are_expected(processors: usize, bus: &BusCounters) {
    let expected: &[u16] = if processors > 1 { &LEGACY_PORTS } else { &[] };
    let unmodelled = bus.unmodelled_ports();
    assert!(
        unmodelled.iter().all(|port| expected.contains(port)),
        "the guest reached unmodelled ports the machine does not expect: {unmodelled:?} of {bus:?}"
    );
}
