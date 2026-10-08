//! Driving a machine to an authenticated Ready, then into the command loop.
//!
//! A cold boot and a restore differ in two places only: how the launch page is published, and
//! how Ready is claimed. Both are handled here so that the command loop below is entered once
//! and cannot drift between the two paths.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use soma_guest::{HostControl, RepairedHostControl};
use soma_kvm::snapshot::readiness::SessionEvidence;
use soma_kvm::x86_64::{Milestone, Restored, SandboxMachine};

use super::super::io::HostIo;
use super::super::pending::PendingActivation;
use super::super::session::{BOOT_DEADLINE, Request, Response, SessionError};
use super::activation::open_network;
use super::commands::serve_commands;
use super::finish::Ending;
use super::inputs::LaunchInputs;

/// Boots a machine from nothing and drives it to Ready, then serves commands.
pub(super) fn drive_cold(
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
    let repaired = super::super::secrets::place(repaired, inputs.secrets)?;
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
    let repaired = super::super::secrets::place(repaired, inputs.secrets)?;

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
        .wait_launch_page_consumed(super::super::io::PAGE_DOMAIN, deadline)
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
