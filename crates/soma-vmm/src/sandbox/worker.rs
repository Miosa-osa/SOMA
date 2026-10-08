//! The body of the thread that owns one machine.
//!
//! Everything here runs on the sandbox thread, which is why the host adapter may borrow the
//! machine: both are locals of the same stack frame for the machine's whole life.
//!
//! A sandbox arrives at Ready one of two ways. A cold boot builds a machine and runs the kernel
//! and userspace init on the request path. A restore resumes a machine captured once for the
//! whole Generation, already past that work. After Ready the two are identical, so the command
//! loop is written once and both paths enter it.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use soma_guest::{HostControl, HostLaunchMaterial, RepairedHostControl, SecretFile};
use soma_kvm::snapshot::readiness::SessionEvidence;
use soma_kvm::x86_64::{Milestone, RestoreRequest, Restored, SandboxMachine, restore};

mod activation;
mod cold;

pub use cold::{ColdBootInputs, config};

use self::activation::open_network;
use super::io::HostIo;

mod commands;
mod files;
mod pty;
use super::pending::PendingActivation;
use super::session::{BOOT_DEADLINE, EXIT_GRACE, Request, Response, SessionError};
use super::source::{Boot, Network, Source};
use commands::serve_commands;

/// How a sandbox that reached Ready stopped serving its owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    /// The guest acknowledged a shutdown and is leaving `KVM_RUN` on its own.
    ShutDown,
    /// The owner abandoned the guest without asking it, so nothing waits for it to leave.
    Aborted,
}

/// What one Instance's launch carries into the machine that will serve it.
///
/// The two travel together because they are consumed together and in one order: the material
/// authenticates the session, and the secrets are the first thing placed over it.
pub struct LaunchInputs<'a> {
    pub material: HostLaunchMaterial,
    pub secrets: &'a [SecretFile],
}

/// Owns one machine for its whole life and answers requests about it.
pub fn serve(boot: Boot, requests: &Receiver<Request>, responses: &Sender<Response>) {
    let Boot {
        source,
        generation,
        instance,
        operation,
        guest_cid,
        network,
        secrets,
    } = boot;
    let Network {
        launch,
        attachment,
        activation,
    } = network;
    let Ok(material) = HostLaunchMaterial::generate(generation, instance, operation, launch) else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };

    match source {
        Source::ColdBoot(config) => {
            let Ok(mut sandbox) = SandboxMachine::create(config) else {
                let _ignored = responses.send(Response::Failed(SessionError::Create));
                return;
            };
            // The frame path is attached before the vCPU runs, so the device thread can watch it
            // from its first wakeup. The link stays down until the assignment is activated.
            if let Some(attachment) = attachment {
                sandbox.attach_network(attachment);
            }
            // The machine is finished on every path out of here, including a failed boot, so no
            // descriptor or thread outlives the sandbox that owned it.
            let inputs = LaunchInputs {
                material,
                secrets: &secrets,
            };
            let outcome = drive_cold(&mut sandbox, inputs, activation, requests, responses);
            report(sandbox, outcome, responses, instance);
        }
        Source::Restore {
            objects,
            hypervisor,
            disks,
            devices,
            memory_bytes,
            vcpus,
            contract,
        } => {
            let restored = restore(RestoreRequest {
                objects,
                hypervisor,
                disks,
                devices,
                guest_cid,
                memory_bytes,
                vcpus,
                contract,
                // Re-hashing every byte of the memory object is the installation and audit
                // boundary, not the request path.
                verify_artifacts: false,
                // An Instance the broker leased a bundle to arrives here with its frame path;
                // one that asked for no egress keeps the device it was built with, whose link
                // stays down and which drops every frame.
                network: attachment,
            });
            let Ok(mut restored) = restored else {
                let _ignored = responses.send(Response::Failed(SessionError::Create));
                return;
            };
            let inputs = LaunchInputs {
                material,
                secrets: &secrets,
            };
            let outcome = drive_restored(
                &mut restored,
                inputs,
                (instance, operation, activation),
                requests,
                responses,
            );
            report(restored.machine, outcome, responses, instance);
        }
    }
}

/// Finishes the machine and reports its evidence, or the failure that ended it.
///
/// An aborted machine is halted and its owner answered before it is finished, so the owner
/// does not wait for the release. An aborted guest is given no exit grace: finishing with a zero deadline kicks the vCPU out of
/// `KVM_RUN` at once, where a guest that acknowledged shutdown is waited on to leave by itself.
pub fn report(
    mut sandbox: SandboxMachine,
    outcome: Result<Ending, SessionError>,
    responses: &Sender<Response>,
    instance: [u8; 16],
) {
    if outcome == Ok(Ending::Aborted) {
        // The owner is answered once the guest can no longer run, and the release it would
        // otherwise wait on, which is closing the VM and returning its memory, happens after.
        sandbox.halt();
        let _ignored = responses.send(Response::Halted);
        drop(sandbox.finish(exit_grace(outcome)));
        return;
    }
    let evidence = sandbox.finish(exit_grace(outcome));
    match outcome {
        Ok(_) => {
            let _ignored = responses.send(Response::Finished(Box::new(evidence)));
        }
        Err(error) => {
            // A failed sandbox never reaches cleanup, so this is the only chance to keep what it
            // recorded. The caller still receives the same typed failure.
            super::timeline::dump_failure(&hex(instance), &evidence, &format!("{error:?}"));
            let _ignored = responses.send(Response::Failed(error));
        }
    }
}

/// How long a finishing machine waits for its vCPU to leave `KVM_RUN` on its own.
///
/// Only a guest that was asked to leave is waited on. An aborted one was never asked, so waiting
/// would spend the whole grace on a guest idling in its command loop before the kick ended it.
const fn exit_grace(outcome: Result<Ending, SessionError>) -> Duration {
    match outcome {
        Ok(Ending::Aborted) => Duration::ZERO,
        Ok(Ending::ShutDown) | Err(_) => EXIT_GRACE,
    }
}

/// The Instance identity as the lowercase hexadecimal the receipt reports.
fn hex(instance: [u8; 16]) -> String {
    use std::fmt::Write as _;
    instance
        .iter()
        .fold(String::with_capacity(32), |mut out, byte| {
            let _ignored = write!(out, "{byte:02x}");
            out
        })
}

/// Boots a machine from nothing and drives it to Ready, then serves commands.
fn drive_cold(
    sandbox: &mut SandboxMachine,
    inputs: LaunchInputs<'_>,
    activation: Option<PendingActivation>,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<Ending, SessionError> {
    let delivered = inputs
        .material
        .deliver_with(|page| sandbox.write_launch_page(page))
        .map_err(|_| SessionError::LaunchPage)?;
    sandbox.start().map_err(|_| SessionError::Create)?;
    let repaired = reach_session(sandbox, delivered)?;
    let repaired = super::secrets::place(repaired, inputs.secrets)?;
    sandbox.mark(Milestone::Ready);
    open_network(sandbox, &repaired, activation, requests, responses)?;
    serve_commands(sandbox, repaired, requests, responses)
}

/// Resumes a captured machine and drives it to Ready, then serves commands.
///
/// A restore differs from a cold boot in two places only. The launch page is published through
/// `resume` rather than written before `start`, and Ready must be claimed with a receipt binding
/// this Instance and operation to the live session transcript, so readiness cannot be asserted by
/// a caller that did not complete the session.
pub fn drive_restored(
    restored: &mut Restored,
    inputs: LaunchInputs<'_>,
    identity: ([u8; 16], [u8; 16], Option<PendingActivation>),
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<Ending, SessionError> {
    let (instance, operation, activation) = identity;
    let delivered = inputs
        .material
        .deliver_with(|page| restored.resume(page))
        .map_err(|_| SessionError::LaunchPage)?;
    let machine = &restored.machine;
    let repaired = reach_session(machine, delivered)?;
    // The snapshot this machine resumed from is shared by every Instance of the Generation, so
    // the secrets are placed after the resume and never appear in the captured state.
    let repaired = super::secrets::place(repaired, inputs.secrets)?;

    let evidence = SessionEvidence::new(instance, operation, repaired.session_transcript())
        .map_err(|_| SessionError::Ready)?;
    let demand = restored.readiness_demand().ok_or(SessionError::Ready)?;
    let receipt = demand.attest(&evidence);
    restored.ready(&receipt).map_err(|_| SessionError::Ready)?;
    machine.mark(Milestone::Ready);

    open_network(machine, &repaired, activation, requests, responses)?;
    serve_commands(machine, repaired, requests, responses)
}

/// The steps both paths share between publishing the launch page and holding a repaired session.
fn reach_session(
    machine: &SandboxMachine,
    delivered: soma_guest::DeliveredHostLaunchMaterial,
) -> Result<RepairedHostControl<HostIo<'_>>, SessionError> {
    let deadline = Instant::now() + BOOT_DEADLINE;
    machine
        .wait_launch_page_consumed(super::io::PAGE_DOMAIN, deadline)
        .map_err(|_| SessionError::LaunchPage)?;
    machine
        .control()
        .wait_connected(deadline)
        .map_err(|_| SessionError::Boot)?;
    machine.mark(Milestone::VsockConnected);
    let host =
        HostControl::connect(delivered, HostIo::new(machine)).map_err(|_| SessionError::Boot)?;
    machine.mark(Milestone::Handshake);
    host.prepare().map_err(|_| SessionError::Ready)
}
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{EXIT_GRACE, Ending, SessionError, exit_grace};

    #[test]
    fn an_aborted_machine_is_kicked_at_once() {
        assert_eq!(exit_grace(Ok(Ending::Aborted)), Duration::ZERO);
    }

    #[test]
    fn a_guest_asked_to_leave_and_a_failed_session_keep_the_exit_grace() {
        assert_eq!(exit_grace(Ok(Ending::ShutDown)), EXIT_GRACE);
        assert_eq!(exit_grace(Err(SessionError::Execute)), EXIT_GRACE);
    }
}
