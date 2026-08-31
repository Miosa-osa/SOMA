//! What a machine prepared ahead of demand does, and what it removes from the request path.
//!
//! One restore is compared with itself. The on-demand sample times the whole `restore` call,
//! which is what a Launch pays today. The prepared sample times only `assign`, which is what a
//! Launch pays once the pool has already paid for `restore_sterile`. Both samples clone their
//! private head outside the timed region, because the head belongs to the Instance either way
//! and preparing a machine cannot remove it.
//!
//! These are single-host, in-container observations of one machine shape. They are inputs to the
//! design, not a certified budget and not a latency claim.

use std::time::{Duration, Instant};

use soma_guest::{HostLaunchMaterial, LaunchNetwork, TerminalStatus};
use soma_kvm::x86_64::{
    Milestone, RestoreRequest, SandboxDisks, Sterile, SterileRequest, restore, restore_sterile,
};

use crate::{
    x86_64_sandbox_boot_host as host, x86_64_sandbox_boot_host::require_kvm,
    x86_64_sandbox_boot_session as session, x86_64_snapshot_restore_fixture as fixture,
    x86_64_snapshot_restore_instance as instance, x86_64_snapshot_restore_report as report,
    x86_64_snapshot_restore_workload as workload,
};

/// Iterations of each arm.
const ITERATIONS: u32 = 10;
/// First context identifier the loop assigns; every restore takes the next one.
const FIRST_CID: u32 = 64;
/// How long the guest has to leave `KVM_RUN` after it acknowledges shutdown.
const EXIT_GRACE: Duration = Duration::from_secs(10);

/// The request-path cost of building a machine, with the pool and without it.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn preparing_a_machine_ahead_of_demand_removes_it_from_the_request_path() {
    require_kvm();
    let fixture = fixture::shared();
    let mut on_demand = Vec::new();
    let mut prepared = Vec::new();

    for iteration in 0..ITERATIONS {
        let cid = FIRST_CID + iteration;

        // On demand: everything the machine costs is paid here, where a request is waiting.
        let (path, head) = fixture.private_head(&format!("on-demand-{iteration}"));
        let started = Instant::now();
        let restored = restore(RestoreRequest {
            paths: fixture.paths.clone(),
            disks: SandboxDisks {
                root: fixture.root(),
                overlay: head,
            },
            guest_cid: cid,
            memory_bytes: fixture.ram_bytes,
            verify_artifacts: false,
            network: None,
        })
        .expect("an on-demand restore produces a machine");
        on_demand.push(elapsed_ns(started));
        drop(restored);
        let _ignored = std::fs::remove_file(&path);

        // Prepared: the same machine, built before the request. Only the transfer of this
        // Instance's head and context identifier remains on the request path.
        let sterile = restore_sterile(SterileRequest {
            paths: fixture.paths.clone(),
            root: fixture.root(),
            overlay_capacity_bytes: fixture.overlay_capacity_bytes(),
            memory_bytes: fixture.ram_bytes,
            verify_artifacts: false,
        })
        .expect("a sterile machine restores without an Instance");
        let (path, head) = fixture.private_head(&format!("prepared-{iteration}"));
        let started = Instant::now();
        let assigned = sterile
            .assign(head, cid, None)
            .expect("a prepared machine accepts one Instance's authority");
        prepared.push(elapsed_ns(started));
        drop(assigned);
        let _ignored = std::fs::remove_file(&path);
    }

    eprintln!("[prepared] machine construction on the request path, ns:");
    report::percentiles("on demand, whole restore", on_demand.clone());
    report::percentiles("prepared, assignment only", prepared.clone());

    let on_demand_median = median(&on_demand);
    let prepared_median = median(&prepared);
    assert!(
        prepared_median < on_demand_median,
        "preparing a machine did not shorten the request path: {prepared_median} ns assigned \
         against {on_demand_median} ns restored"
    );
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// The nearest-rank median, which is what the retained percentiles report.
fn median(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied().unwrap_or(0)
}

/// One machine prepared before any Instance existed.
fn sterile() -> Sterile {
    let fixture = fixture::shared();
    restore_sterile(SterileRequest {
        paths: fixture.paths.clone(),
        root: fixture.root(),
        overlay_capacity_bytes: fixture.overlay_capacity_bytes(),
        memory_bytes: fixture.ram_bytes,
        verify_artifacts: false,
    })
    .expect("a sterile machine restores without an Instance")
}

/// A machine restored before its Instance existed still reaches an authenticated session and
/// runs a command, and releases everything it held afterwards.
///
/// This is the path a claimed prepared worker takes, so it is the proof that the pool serves a
/// working machine rather than only a fast one.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_prepared_machine_reaches_ready_and_runs_one_command() {
    require_kvm();
    let descriptors_before = host::open_descriptor_count();
    let cid = FIRST_CID + ITERATIONS;
    let prepared = sterile();
    let fixture = fixture::shared();
    let (head_path, head) = fixture.private_head("prepared-ready");

    let mut restored = prepared
        .assign(head, cid, None)
        .expect("a prepared machine accepts one Instance's authority");

    let identity = instance::Identity {
        instance: session::random16(),
        operation: session::random16(),
    };
    let network = LaunchNetwork::new(
        cid,
        cid,
        restored.facts.mac,
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        session::now_unix_nanos(),
    )
    .expect("link-down placeholder network");
    let material = HostLaunchMaterial::generate(
        fixture.candidate_id,
        identity.instance,
        identity.operation,
        network,
    )
    .expect("fresh launch material");
    let delivered = material
        .deliver_with(|page| restored.resume(page))
        .expect("resume the assigned machine");

    let commands = [instance::command(b"/usr/local/bin/node", &[b"--version"])];
    let mut work = workload::Commands(&commands);
    let outcome = instance::drive(&restored, delivered, &identity, &mut work);
    let complete = restored.is_ready();
    let evidence = restored.machine.finish(EXIT_GRACE);
    let (executed, _transcript) = outcome.unwrap_or_else(|error| {
        panic!(
            "a prepared machine failed its session: {error}; exit={:?}",
            evidence.exit
        )
    });

    assert!(complete, "the prepared restore skipped an ordered step");
    assert_eq!(executed[0].status, TerminalStatus::Exited(0));
    assert!(
        String::from_utf8_lossy(&executed[0].stdout).starts_with("v22."),
        "a prepared machine did not report the Node version"
    );
    assert!(evidence.launch_page_retired);
    assert!(evidence.at(Milestone::Ready).is_some());
    let _ignored = std::fs::remove_file(&head_path);
    assert_eq!(
        host::open_descriptor_count(),
        descriptors_before,
        "a finished prepared machine leaked descriptors"
    );
}
