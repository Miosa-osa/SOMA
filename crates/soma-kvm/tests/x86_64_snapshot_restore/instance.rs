//! Restoring one Instance from the shared snapshot and driving it to an executed command.
//!
//! Everything after the resume is the production path: the guest agent consumes a launch page
//! it has never seen, repairs its entropy, identity, and network state, authenticates over a
//! fresh vsock endpoint, answers the fixed readiness probe, and only then executes.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use soma_guest::{
    GuestCommand, HostControl, HostLaunchMaterial, LaunchNetwork, OperationId, ResponderPublicKey,
};
use soma_kvm::x86_64::{
    Milestone, RestoreFacts, RestoreRequest, SandboxDisks, SandboxEvidence, restore,
};

use crate::{
    x86_64_sandbox_boot_control::HostIo,
    x86_64_sandbox_boot_session as session,
    x86_64_snapshot_restore_fixture::{self as fixture, Fixture},
};

const EXIT_GRACE: Duration = Duration::from_secs(10);

/// What one restored Instance left behind.
pub struct Instance {
    pub evidence: SandboxEvidence,
    pub executed: Vec<session::Executed>,
    pub facts: RestoreFacts,
    pub identity: [u8; 16],
    pub head_path: PathBuf,
    /// Nanoseconds from the first manifest byte read to the machine being ready to resume.
    pub restore_ns: u64,
}

/// Restores one Instance, runs `command`, shuts it down, and returns the evidence.
///
/// # Panics
///
/// Panics with the session failure; a restored Instance that cannot reach `Ready` is the
/// result this test exists to catch.
pub fn run(fixture: &Fixture, name: &str, cid: u32, commands: &[session::Command<'_>]) -> Instance {
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
    })
    .expect("restore the snapshot");
    let restore_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let facts = restored.facts.clone();
    assert_eq!(facts.repair_point_line, fixture::REPAIR_POINT_LINE);
    assert_eq!(facts.captured_cid, u64::from(fixture::CAPTURE_CID));

    let instance_id = session::random16();
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
    let material = HostLaunchMaterial::generate(
        fixture.generation_id,
        instance_id,
        session::random16(),
        network,
    )
    .expect("fresh launch material");
    let delivered = material
        .deliver_with(|page| restored.resume(page))
        .expect("resume the restored machine");

    let responder =
        ResponderPublicKey::new(fixture.responder_public).expect("the pinned responder key");
    let outcome = drive(&restored, delivered, &responder, commands);
    let complete = restored.is_ready();
    let evidence = restored.machine.finish(EXIT_GRACE);
    let executed = match outcome {
        Ok(executed) => executed,
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
        executed,
        facts,
        identity: instance_id,
        head_path,
        restore_ns,
    }
}

fn drive(
    restored: &soma_kvm::x86_64::Restored,
    delivered: soma_guest::DeliveredHostLaunchMaterial,
    responder: &ResponderPublicKey,
    commands: &[session::Command<'_>],
) -> Result<Vec<session::Executed>, String> {
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
    let host = HostControl::connect(delivered, responder, HostIo::new(machine))
        .map_err(|error| format!("handshake: {error}"))?;
    machine.mark(Milestone::Handshake);
    let repaired = host
        .prepare_and_probe()
        .map_err(|error| format!("repair and probe: {error}"))?;
    restored
        .ready()
        .map_err(|error| format!("ready: {error}"))?;
    machine.mark(Milestone::Ready);
    let mut repaired = repaired;
    let mut executed = Vec::with_capacity(commands.len());
    for (index, command) in commands.iter().enumerate() {
        let guest_command = GuestCommand::new(
            command.program.to_vec(),
            command.arguments.iter().map(|arg| arg.to_vec()).collect(),
            command.timeout_millis,
            command.output_bytes,
        )
        .map_err(|error| format!("command: {error}"))?;
        let (next, outcome) = repaired
            .execute(
                OperationId::new(session::random16()).unwrap(),
                guest_command,
            )
            .map_err(|error| format!("execute: {error}"))?;
        repaired = next;
        if index == 0 {
            // The warm timeline measures the first command only; later ones are assertions.
            machine.mark(Milestone::Execute);
        }
        executed.push(session::Executed {
            status: outcome.status(),
            stdout: outcome.stdout().to_vec(),
            stderr: outcome.stderr().to_vec(),
        });
    }
    repaired
        .shutdown(OperationId::new(session::random16()).unwrap())
        .map_err(|error| format!("shutdown: {error}"))?;
    machine.mark(Milestone::Shutdown);
    Ok(executed)
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
