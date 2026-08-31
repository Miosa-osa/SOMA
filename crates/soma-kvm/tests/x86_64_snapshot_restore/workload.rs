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

/// The authenticated repaired session, borrowed from the live machine that carries it.
pub type Session<'a> = RepairedHostControl<HostIo<'a>>;

/// One body of work driven over a ready session.
///
/// The lifetime sits on the method rather than on the trait because the session borrows the
/// machine it runs on, and a workload value is built before that machine exists.
pub trait Workload {
    /// What the workload retains for the test that asked for it.
    type Output;

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
