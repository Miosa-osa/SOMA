//! Restoring one Instance from the shared snapshot and driving one workload on it.
//!
//! Everything after the resume is the production path: the guest agent consumes a launch page
//! it has never seen, repairs its entropy, identity, and network state, authenticates over a
//! fresh vsock endpoint, answers the fixed readiness probe, and only then serves the workload.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use soma_guest::{HostControl, HostLaunchMaterial, LaunchNetwork, OperationId};
use soma_kvm::snapshot::readiness::{ReadinessRefusal, SessionEvidence};
use soma_kvm::x86_64::{
    Milestone, RestoreFacts, RestoreRequest, SandboxDisks, SandboxEvidence, SnapshotError, restore,
};

use crate::{
    x86_64_sandbox_boot_control::HostIo,
    x86_64_sandbox_boot_host as host, x86_64_sandbox_boot_session as session,
    x86_64_snapshot_restore_fixture::{self as fixture, Fixture},
    x86_64_snapshot_restore_workload::{self as workload, Workload},
};

const EXIT_GRACE: Duration = Duration::from_secs(10);

/// What one restored Instance left behind, with whatever its workload retained.
pub struct Instance<T> {
    pub evidence: SandboxEvidence,
    /// What the workload produced; a command list produces its ordered results.
    pub output: T,
    pub facts: RestoreFacts,
    pub identity: [u8; 16],
    /// The digest of the authenticated session the readiness receipt was minted from.
    pub session_transcript: [u8; 32],
    pub head_path: PathBuf,
    /// Nanoseconds from the first manifest byte read to the machine being ready to resume.
    pub restore_ns: u64,
    /// Open descriptors and live threads before and after the restored Instance existed.
    pub descriptors: (usize, usize),
    pub threads: (u64, u64),
}

/// Restores one Instance, runs the ordered commands, shuts it down, and returns the evidence.
///
/// # Panics
///
/// Panics with the session failure; a restored Instance that cannot reach `Ready` is the
/// result this test exists to catch.
pub fn run(
    fixture: &Fixture,
    name: &str,
    cid: u32,
    commands: &[session::Command<'_>],
) -> Instance<Vec<session::Executed>> {
    run_workload(fixture, name, cid, workload::Commands(commands))
}

/// Restores one Instance, runs one workload over its ready session, and shuts it down.
///
/// # Panics
///
/// Panics with the session failure, for the reason [`run`] gives.
pub fn run_workload<W: Workload>(
    fixture: &Fixture,
    name: &str,
    cid: u32,
    mut work: W,
) -> Instance<W::Output> {
    // Every descriptor and thread the Instance uses, including the private head handed to
    // the restore, is opened after these counts and must be gone before the ones taken at the
    // end.
    let descriptors_before = host::open_descriptor_count();
    let threads_before = host::thread_count();
    let (head_path, head) = fixture.private_head(name);
    let started = Instant::now();
    let mut restored = restore(RestoreRequest {
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
    .expect("restore the snapshot");
    let restore_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let facts = restored.facts.clone();
    assert_eq!(facts.repair_point_line, fixture::REPAIR_POINT_LINE);
    assert_eq!(facts.captured_cid, u64::from(fixture::CAPTURE_CID));

    let instance_id = session::random16();
    let launch_operation = session::random16();
    let network = LaunchNetwork::new(
        cid,
        cid,
        facts.mac,
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        session::now_unix_nanos(),
    )
    .expect("link-down placeholder network");
    let material =
        HostLaunchMaterial::generate(fixture.candidate_id, instance_id, launch_operation, network)
            .expect("fresh launch material");
    let delivered = material
        .deliver_with(|page| restored.resume(page))
        .expect("resume the restored machine");

    let outcome = drive(
        &restored,
        delivered,
        &Identity {
            instance: instance_id,
            operation: launch_operation,
        },
        &mut work,
    );
    let complete = restored.is_ready();
    let evidence = restored.machine.finish(EXIT_GRACE);
    let log = fixture.scratch.join(format!("restore-{name}.log"));
    std::fs::write(&log, &evidence.serial).expect("retain the restored console");
    let console = String::from_utf8_lossy(&evidence.serial);
    eprintln!(
        "[{name}] console ({} bytes) retained at {}:",
        evidence.serial.len(),
        log.display()
    );
    for line in console
        .lines()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .iter()
        .rev()
    {
        eprintln!("  | {line}");
    }
    let (output, session_transcript) = match outcome {
        Ok(finished) => finished,
        Err(error) => {
            panic!(
                "[{name}] restored session failed: {error}; exit={:?}",
                evidence.exit
            )
        }
    };
    assert!(
        complete,
        "the restore sequence did not complete every ordered step"
    );
    Instance {
        evidence,
        output,
        facts,
        identity: instance_id,
        session_transcript,
        head_path,
        restore_ns,
        descriptors: (descriptors_before, host::open_descriptor_count()),
        threads: (threads_before, host::thread_count()),
    }
}

/// The fresh identity the launch authority named, which the readiness receipt must bind.
pub struct Identity {
    pub instance: [u8; 16],
    pub operation: [u8; 16],
}

pub fn drive<W: Workload>(
    restored: &soma_kvm::x86_64::Restored,
    delivered: soma_guest::DeliveredHostLaunchMaterial,
    identity: &Identity,
    work: &mut W,
) -> Result<(W::Output, [u8; 32]), String> {
    let machine = &restored.machine;
    let deadline = Instant::now() + session::BOOT_DEADLINE;
    machine
        .wait_launch_page_consumed(session::PAGE_DOMAIN, deadline)
        .map_err(|error| format!("launch page consumption: {error}"))?;
    machine
        .control()
        .wait_connected(deadline)
        .map_err(|error| format!("vsock connection: {error}"))?;
    machine.mark(Milestone::VsockConnected);
    let host = HostControl::connect(delivered, HostIo::new(machine))
        .map_err(|error| format!("handshake: {error}"))?;
    machine.mark(Milestone::Handshake);
    let repaired = host
        .prepare_and_probe()
        .map_err(|error| format!("repair and probe: {error}"))?;
    let transcript = repaired.session_transcript();
    let evidence = SessionEvidence::new(identity.instance, identity.operation, transcript)
        .map_err(|error| format!("session evidence: {error}"))?;
    let demand = restored
        .readiness_demand()
        .ok_or_else(|| "the restore published no readiness demand".to_owned())?;
    let receipt = demand.attest(&evidence);
    restored
        .ready(&receipt)
        .map_err(|error| format!("ready: {error}"))?;
    assert!(
        matches!(
            restored.ready(&receipt),
            Err(SnapshotError::Readiness(ReadinessRefusal::Spent))
        ),
        "a spent readiness challenge accepted a second receipt"
    );
    machine.mark(Milestone::Ready);
    // A receipt that is never printed cannot be scanned for a value it must not carry, so the
    // one thing about it that varies with the session travels back to the caller instead.
    eprintln!(
        "[receipt] {receipt:?} over session transcript {}",
        hex(&transcript)
    );
    let (repaired, output) = work.run(machine, repaired)?;
    repaired
        .shutdown(OperationId::new(session::random16()).unwrap())
        .map_err(|error| format!("shutdown: {error}"))?;
    machine.mark(Milestone::Shutdown);
    Ok((output, transcript))
}

/// One bounded command with the fixed test budgets.
#[must_use]
pub fn command<'a>(program: &'a [u8], arguments: &'a [&'a [u8]]) -> session::Command<'a> {
    session::Command {
        program,
        arguments,
        timeout_millis: 30_000,
        output_bytes: 65_536,
    }
}

/// Renders bytes for a log line that must never carry tenant data.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        write!(text, "{byte:02x}").expect("write to a string");
        text
    })
}
