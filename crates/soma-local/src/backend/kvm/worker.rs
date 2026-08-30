//! The body of the thread that owns one machine.
//!
//! Everything here runs on the sandbox thread, which is why the host adapter may borrow the
//! machine: both are locals of the same stack frame for the machine's whole life.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use soma_guest::{HostControl, HostLaunchMaterial, OperationId};
use soma_kvm::x86_64::DeviceIdentity;
use soma_kvm::x86_64::{Milestone, SandboxConfig, SandboxDisks, SandboxMachine};

use super::io::HostIo;
use super::session::{
    BOOT_DEADLINE, Boot, Completed, EXIT_GRACE, GUEST_CID, GUEST_MAC, Request, Response,
    SessionError,
};

/// Owns one machine for its whole life and answers requests about it.
pub(super) fn serve(boot: Boot, requests: &Receiver<Request>, responses: &Sender<Response>) {
    let Boot {
        config,
        generation,
        instance,
        machine,
        network,
    } = boot;
    let Ok(material) = HostLaunchMaterial::generate(generation, instance, machine, network) else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };
    let Ok(mut sandbox) = SandboxMachine::create(config) else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };
    // The machine is finished on every path out of this function, including a failed boot, so
    // no descriptor or thread outlives the sandbox that owned it.
    let outcome = drive(&mut sandbox, material, requests, responses);
    let evidence = sandbox.finish(EXIT_GRACE);
    match outcome {
        Ok(()) => {
            let _ignored = responses.send(Response::Finished(Box::new(evidence)));
        }
        Err(error) => {
            let _ignored = responses.send(Response::Failed(error));
        }
    }
}

fn drive(
    sandbox: &mut SandboxMachine,
    material: HostLaunchMaterial,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<(), SessionError> {
    let delivered = material
        .deliver_with(|page| sandbox.write_launch_page(page))
        .map_err(|_| SessionError::LaunchPage)?;
    sandbox.start().map_err(|_| SessionError::Create)?;
    let deadline = Instant::now() + BOOT_DEADLINE;
    sandbox
        .wait_launch_page_consumed(super::io::PAGE_DOMAIN, deadline)
        .map_err(|_| SessionError::LaunchPage)?;
    sandbox
        .control()
        .wait_connected(deadline)
        .map_err(|_| SessionError::Boot)?;
    sandbox.mark(Milestone::VsockConnected);
    let host =
        HostControl::connect(delivered, HostIo::new(sandbox)).map_err(|_| SessionError::Boot)?;
    sandbox.mark(Milestone::Handshake);
    let mut repaired = host.prepare_and_probe().map_err(|_| SessionError::Ready)?;
    sandbox.mark(Milestone::Ready);
    responses
        .send(Response::Ready)
        .map_err(|_| SessionError::Gone)?;

    // A closed request channel is an ordinary end: the owner dropped the session, so the guest
    // is shut down exactly as an explicit shutdown would.
    while let Ok(request) = requests.recv() {
        match request {
            Request::Execute(command) => {
                let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
                let (next, outcome) = repaired
                    .execute(operation, command)
                    .map_err(|_| SessionError::Execute)?;
                repaired = next;
                sandbox.mark(Milestone::Execute);
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
    sandbox.mark(Milestone::Shutdown);
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
) -> SandboxConfig {
    SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks { root, overlay },
        identity: DeviceIdentity {
            guest_cid: GUEST_CID,
            guest_mac: GUEST_MAC,
        },
        ram_bytes,
    }
}
