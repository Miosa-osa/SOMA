//! What one restored Instance is asked to do once its session is Ready.
//!
//! The restore, the launch material, the handshake, the repair, and the readiness receipt are
//! identical for every live proof in this suite, so they stay in
//! [`crate::x86_64_snapshot_restore_instance`] and only the work after `Ready` varies. A
//! workload is that work: it is handed the one reusable session owner and must hand it back, so
//! a proof cannot leave the session in a state the shutdown that follows it would not accept.

use soma_guest::{GuestCommand, OperationId, RepairedHostControl};
use soma_kvm::x86_64::{Milestone, SandboxMachine};

use crate::{x86_64_sandbox_boot_control::HostIo, x86_64_sandbox_boot_session as session};
#[path = "workload/resident_steps.rs"]
mod resident_steps;
use resident_steps::{exercise_resident, find_in_image};

/// The authenticated repaired session, borrowed from the live machine that carries it.
pub type Session<'a> = RepairedHostControl<HostIo<'a>>;

/// One body of work driven over a ready session.
///
/// The lifetime sits on the method rather than on the trait because the session borrows the
/// machine it runs on, and a workload value is built before that machine exists.
pub trait Workload {
    /// What the workload retains for the test that asked for it.
    type Output;

    fn bind_instance(&mut self, _instance_id: [u8; 16]) {}

    /// Runs against the ready session and returns the reusable owner with the result.
    ///
    /// # Errors
    ///
    /// Returns the failure text the calling test panics with.
    fn run<'a>(
        &mut self,
        machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Self::Output), String>;
}

/// The ordered bounded-command list the snapshot proofs drive.
pub struct Commands<'c>(pub &'c [session::Command<'c>]);

impl Workload for Commands<'_> {
    type Output = Vec<session::Executed>;

    fn run<'a>(
        &mut self,
        machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Vec<session::Executed>), String> {
        let mut session = session;
        let mut executed = Vec::with_capacity(self.0.len());
        for (index, command) in self.0.iter().enumerate() {
            let (next, outcome) = execute(session, command)?;
            session = next;
            if index == 0 {
                // The warm timeline measures the first command only; later ones are assertions.
                machine.mark(Milestone::Execute);
            }
            executed.push(outcome);
        }
        Ok((session, executed))
    }
}

/// Executes the runtime's binary before pausing the authenticated machine in place.
pub struct PauseAfterCommand;

impl Workload for PauseAfterCommand {
    type Output = session::Executed;

    fn run<'a>(
        &mut self,
        machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Self::Output), String> {
        let command = super::x86_64_snapshot_restore_instance::command(
            b"/usr/local/bin/node",
            &[b"--version"],
        );
        let (session, executed) = execute(session, &command)?;
        machine
            .pause(std::time::Duration::from_secs(2))
            .map_err(|error| format!("pause: {error}"))?;
        assert!(machine.is_paused());
        Ok((session, executed))
    }
}

/// Runs a detached Linux process whose counter and marker exist only in its memory.
pub struct ResumeResidentProcess;

impl Workload for ResumeResidentProcess {
    type Output = (u64, u64);

    fn run<'a>(
        &mut self,
        machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Self::Output), String> {
        exercise_resident(machine, session, |machine, _marker| {
            use std::time::Duration;
            machine
                .pause(Duration::from_secs(2))
                .map_err(|error| format!("pause: {error}"))?;
            assert!(machine.is_paused());
            machine
                .resume()
                .map_err(|error| format!("resume: {error}"))?;
            machine
                .resume()
                .map_err(|error| format!("repeat resume: {error}"))?;
            Ok(())
        })
    }
}

/// Captures the original Instance while the same authenticated workload is active.
pub struct CheckpointResidentProcess {
    pub instance_id: [u8; 16],
    pub candidate_id: [u8; 32],
    pub root_digest: soma_kvm::snapshot::Digest,
    pub contract: soma_kvm::MachineContract,
}

impl Workload for CheckpointResidentProcess {
    type Output = (u64, u64);

    fn bind_instance(&mut self, instance_id: [u8; 16]) {
        self.instance_id = instance_id;
    }

    fn run<'a>(
        &mut self,
        machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Self::Output), String> {
        use soma_kvm::{
            snapshot::manifest::{CheckpointManifest, Manifest},
            x86_64::{CheckpointRequest, CheckpointTargets, capture_checkpoint},
        };
        use std::{
            io::{Read as _, Seek as _},
            time::Duration,
        };

        let instance_id = self.instance_id;
        exercise_resident(machine, session, |machine, marker| {
            let mut state = tempfile::tempfile().expect("private state target");
            let mut memory = tempfile::tempfile().expect("private memory target");
            let overlay = tempfile::tempfile().expect("private overlay target");
            let started = std::time::Instant::now();
            let manifest = capture_checkpoint(
                machine,
                &CheckpointRequest {
                    instance_id,
                    candidate_id: self.candidate_id,
                    root_digest: self.root_digest,
                    contract: self.contract,
                    grace: Duration::from_secs(2),
                    targets: CheckpointTargets {
                        state: &state,
                        memory: &memory,
                        overlay: Some(&overlay),
                        volume: None,
                    },
                },
            )
            .map_err(|error| format!("checkpoint: {error}"))?;
            let mut encoded = Vec::new();
            state.rewind().expect("rewind state");
            state
                .read_to_end(&mut encoded)
                .expect("read checkpoint state");
            assert_eq!(CheckpointManifest::decode(&encoded).unwrap(), manifest);
            assert!(Manifest::decode(&encoded).is_err());
            assert_eq!(manifest.instance_id(), &instance_id);
            assert_eq!(
                memory.metadata().unwrap().len(),
                manifest.header().memory.size()
            );
            assert!(overlay.metadata().unwrap().len() > 0);
            memory.rewind().expect("rewind checkpoint RAM");
            assert!(
                find_in_image(&mut memory, marker.as_bytes()),
                "the guest's private marker was missing from checkpoint RAM"
            );
            eprintln!(
                "[resident-checkpoint] capture and verification took {} ms",
                started.elapsed().as_millis()
            );
            eprintln!(
                "[resident-checkpoint] verified private files: state={} bytes, RAM={} bytes, overlay={} bytes",
                encoded.len(),
                manifest.header().memory.size(),
                overlay.metadata().unwrap().len()
            );
            Ok(())
        })
    }
}

/// Runs one bounded command over the session and retains its typed result.
///
/// # Errors
///
/// Returns the failure text the calling test panics with.
pub fn execute<'a>(
    session: Session<'a>,
    command: &session::Command<'_>,
) -> Result<(Session<'a>, session::Executed), String> {
    let guest_command = GuestCommand::new(
        command.program.to_vec(),
        command.arguments.iter().map(|arg| arg.to_vec()).collect(),
        command.timeout_millis,
        command.output_bytes,
    )
    .map_err(|error| format!("command: {error}"))?;
    run_command(session, guest_command)
}

/// Runs one already assembled command, including one that carries a context.
///
/// # Errors
///
/// Returns the failure text the calling test panics with.
pub fn run_command(
    session: Session<'_>,
    command: GuestCommand,
) -> Result<(Session<'_>, session::Executed), String> {
    let operation = OperationId::new(session::random16())
        .map_err(|error| format!("operation identity: {error}"))?;
    let (session, outcome) = session
        .execute(operation, command)
        .map_err(|error| format!("execute: {error}"))?;
    Ok((
        session,
        session::Executed {
            status: outcome.status(),
            stdout: outcome.stdout().to_vec(),
            stderr: outcome.stderr().to_vec(),
        },
    ))
}
