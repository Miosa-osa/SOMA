//! Human-readable evidence emitted by the live sandbox proof.

use std::{fs, path::Path};

use soma_kvm::x86_64::SandboxEvidence;

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
}
