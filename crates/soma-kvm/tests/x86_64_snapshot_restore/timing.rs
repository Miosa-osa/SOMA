//! The warm restore loop: ten sequential restores of one snapshot with raw per-milestone
//! samples and nearest-rank percentiles.
//!
//! These are debug-build, single-host, in-container observations of one machine shape. They
//! are inputs to the design, not a certified budget and not a latency claim.

use std::fs;

use soma_guest::TerminalStatus;
use soma_kvm::x86_64::GuestExit;

use crate::{
    x86_64_sandbox_boot_host::require_kvm, x86_64_snapshot_restore_fixture as fixture,
    x86_64_snapshot_restore_instance as instance, x86_64_snapshot_restore_report as report,
};

/// Iterations of the warm restore loop.
const ITERATIONS: u32 = 10;
/// First context identifier used by the loop; every iteration takes the next one.
const FIRST_CID: u32 = 16;

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn warm_restore_timing_over_ten_iterations() {
    require_kvm();
    let Some(fixture) = fixture::shared() else {
        return fixture::skip();
    };
    let commands = [instance::command(b"/usr/local/bin/node", &[b"--version"])];
    let milestones: Vec<(soma_kvm::x86_64::Milestone, &str)> =
        report::WARM.into_iter().chain(report::AFTER).collect();
    let mut samples: Vec<Vec<u64>> = vec![Vec::new(); milestones.len()];
    let mut restore_ns = Vec::new();

    for iteration in 0..ITERATIONS {
        let name = format!("loop-{iteration}");
        let restored = instance::run(&fixture, &name, FIRST_CID + iteration, &commands);
        assert_eq!(restored.executed[0].status, TerminalStatus::Exited(0));
        assert_eq!(restored.evidence.exit, Ok(GuestExit::Reset));
        assert!(
            String::from_utf8_lossy(&restored.executed[0].stdout).starts_with("v22."),
            "iteration {iteration} did not report the Node version"
        );
        if iteration == 0 {
            report::timeline(&name, &restored.evidence);
        }
        restore_ns.push(restored.restore_ns);
        for (index, (milestone, _)) in milestones.iter().enumerate() {
            if let Some(at) = restored.evidence.at(*milestone) {
                samples[index].push(at);
            }
        }
        let _ignored = fs::remove_file(&restored.head_path);
    }

    eprintln!("[loop] WARM percentiles over {ITERATIONS} iterations, ns since the restore began:");
    for (index, (_, name)) in milestones.iter().enumerate() {
        report::percentiles(name, samples[index].clone());
    }
    report::percentiles("restore call, host side", restore_ns);
}
