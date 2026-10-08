//! The evidence table one live sandbox run prints, and what its counters must say.

use std::{fs, path::Path};

use soma_kvm::x86_64::{ExitReason, SandboxEvidence};

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
