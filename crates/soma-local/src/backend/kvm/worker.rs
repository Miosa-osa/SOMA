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
use std::time::Instant;

use soma_guest::{HostControl, HostLaunchMaterial, OperationId, RepairedHostControl};
use soma_kvm::snapshot::readiness::SessionEvidence;
use soma_kvm::x86_64::{
    DeviceIdentity, Milestone, RestoreRequest, Restored, SandboxConfig, SandboxDisks,
    SandboxMachine, SnapshotPaths, restore,
};

use super::io::HostIo;
use super::session::{
    BOOT_DEADLINE, Boot, Completed, EXIT_GRACE, GUEST_MAC, Request, Response, SessionError, Source,
};

/// Owns one machine for its whole life and answers requests about it.
pub(super) fn serve(boot: Boot, requests: &Receiver<Request>, responses: &Sender<Response>) {
    let Boot {
        source,
        generation,
        instance,
        operation,
        guest_cid,
        network,
    } = boot;
    let Ok(material) = HostLaunchMaterial::generate(generation, instance, operation, network)
    else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };

    match source {
        Source::ColdBoot(config) => {
            let Ok(mut sandbox) = SandboxMachine::create(config) else {
                let _ignored = responses.send(Response::Failed(SessionError::Create));
                return;
            };
            // The machine is finished on every path out of here, including a failed boot, so no
            // descriptor or thread outlives the sandbox that owned it.
            let outcome = drive_cold(&mut sandbox, material, requests, responses);
            report(sandbox, outcome, responses, instance);
        }
        Source::Restore {
            snapshot,
            disks,
            memory_bytes,
        } => {
            let restored = restore(RestoreRequest {
                paths: SnapshotPaths::new(snapshot),
                disks,
                guest_cid,
                memory_bytes,
                // Re-hashing every byte of the memory object is the installation and audit
                // boundary, not the request path.
                verify_artifacts: false,
                // No network bundle is assigned yet, so the guest keeps the device it was built
                // with and the link stays down. This is where an admitted bundle will arrive.
                network: None,
            });
            let Ok(mut restored) = restored else {
                let _ignored = responses.send(Response::Failed(SessionError::Create));
                return;
            };
            let outcome = drive_restored(
                &mut restored,
                material,
                instance,
                operation,
                requests,
                responses,
            );
            report(restored.machine, outcome, responses, instance);
        }
    }
}

/// Finishes the machine and reports its evidence, or the failure that ended it.
fn report(
    sandbox: SandboxMachine,
    outcome: Result<(), SessionError>,
    responses: &Sender<Response>,
    instance: [u8; 16],
) {
    let evidence = sandbox.finish(EXIT_GRACE);
    match outcome {
        Ok(()) => {
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
    material: HostLaunchMaterial,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<(), SessionError> {
    let delivered = material
        .deliver_with(|page| sandbox.write_launch_page(page))
        .map_err(|_| SessionError::LaunchPage)?;
    sandbox.start().map_err(|_| SessionError::Create)?;
    let repaired = reach_session(sandbox, delivered)?;
    sandbox.mark(Milestone::Ready);
    serve_commands(sandbox, repaired, requests, responses)
}

/// Resumes a captured machine and drives it to Ready, then serves commands.
///
/// A restore differs from a cold boot in two places only. The launch page is published through
/// `resume` rather than written before `start`, and Ready must be claimed with a receipt binding
/// this Instance and operation to the live session transcript, so readiness cannot be asserted by
/// a caller that did not complete the session.
fn drive_restored(
    restored: &mut Restored,
    material: HostLaunchMaterial,
    instance: [u8; 16],
    operation: [u8; 16],
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<(), SessionError> {
    let delivered = material
        .deliver_with(|page| restored.resume(page))
        .map_err(|_| SessionError::LaunchPage)?;
    let machine = &restored.machine;
    let repaired = reach_session(machine, delivered)?;

    let evidence = SessionEvidence::new(instance, operation, repaired.session_transcript())
        .map_err(|_| SessionError::Ready)?;
    let demand = restored.readiness_demand().ok_or(SessionError::Ready)?;
    let receipt = demand.attest(&evidence);
    restored.ready(&receipt).map_err(|_| SessionError::Ready)?;
    machine.mark(Milestone::Ready);

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
    host.prepare_and_probe().map_err(|_| SessionError::Ready)
}

/// Announces Ready and serves bounded commands until the owner shuts the sandbox down.
fn serve_commands(
    machine: &SandboxMachine,
    mut repaired: RepairedHostControl<HostIo<'_>>,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<(), SessionError> {
    responses
        .send(Response::Ready)
        .map_err(|_| SessionError::Gone)?;

    // A closed request channel is an ordinary end: the owner dropped the session, so the guest is
    // shut down exactly as an explicit shutdown would.
    while let Ok(request) = requests.recv() {
        match request {
            Request::Execute(command) => {
                let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
                let (next, outcome) = repaired
                    .execute(operation, command)
                    .map_err(|_| SessionError::Execute)?;
                repaired = next;
                machine.mark(Milestone::Execute);
                responses
                    .send(Response::Executed(Box::new(Completed {
                        status: outcome.status(),
                        stdout: outcome.stdout().to_vec(),
                        stderr: outcome.stderr().to_vec(),
                    })))
                    .map_err(|_| SessionError::Gone)?;
            }
            Request::Shutdown => break,
        }
    }
    let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
    repaired
        .shutdown(operation)
        .map_err(|_| SessionError::Gone)?;
    machine.mark(Milestone::Shutdown);
    Ok(())
}

/// Sixteen fresh bytes for one operation identity.
fn fresh16() -> [u8; 16] {
    use std::io::Read as _;
    let mut bytes = [0_u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ignored = file.read_exact(&mut bytes);
    }
    bytes
}

/// The device identity and shape one sandbox is given.
pub(super) fn config(
    kernel: std::fs::File,
    initramfs: std::fs::File,
    root: std::fs::File,
    overlay: std::fs::File,
    ram_bytes: u64,
    guest_cid: u32,
) -> SandboxConfig {
    SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks { root, overlay },
        identity: DeviceIdentity {
            guest_cid,
            guest_mac: GUEST_MAC,
        },
        ram_bytes,
    }
}
