//! The bounded lifecycle the worker serves over its control descriptor.
//!
//! One packet is one request and one reply, served in arrival order on the only thread the
//! worker has. Three things bound the session: a packet may not exceed
//! [`MAX_REQUEST_BYTES`], a session may not exceed [`MAX_REQUESTS`] packets, and every request
//! that decodes is one the [`Machine`] itself admits or refuses, so a supervisor cannot spend
//! the worker's memory or drive it outside its own state machine.
//!
//! The session always ends, and its exit status says which way: on a stop the Machine
//! completed, on a shutdown request, when the control socket reaches end of stream because the
//! supervisor is gone, or when the request budget runs out.

use soma_jail::{DescriptorManifest, DescriptorRole, Phase, attest, install_filter};
use soma_vmm::{
    MAX_OPERATION_RECEIPTS, Machine,
    control::{MAX_REQUEST_BYTES, Reply, Request},
};

use crate::{channel::Channel, exit};

/// The request budget of one worker life.
///
/// The Machine retains at most [`MAX_OPERATION_RECEIPTS`] terminal receipts, so a supervisor
/// that has sent that many packets has either finished or is no longer following the contract.
const MAX_REQUESTS: usize = MAX_OPERATION_RECEIPTS;

/// Serves requests until the lifecycle ends, and returns the worker's exit status.
pub fn serve(control: &Channel, manifest: &DescriptorManifest) -> i32 {
    // A manifest that names a hypervisor is a machine this worker is expected to hold, so a
    // table that names one and is then missing a piece of it refuses service rather than
    // quietly serving the contract with nothing behind it. A manifest that names no hypervisor
    // is the jail's own containment shape, which has no machine to hold and never asks for one.
    let mut machine = if manifest.roles().contains(&DescriptorRole::Kvm) {
        match Machine::on_jailed_kvm(manifest) {
            Some(machine) => machine,
            None => return exit::NO_MANIFEST,
        }
    } else {
        Machine::new()
    };
    let mut buffer = [0u8; MAX_REQUEST_BYTES];
    for _ in 0..MAX_REQUESTS {
        let Some(count) = control.receive(&mut buffer) else {
            return exit::SUPERVISOR_GONE;
        };
        let packet = String::from_utf8_lossy(&buffer[..count]).into_owned();
        let request = match Request::decode(&packet) {
            Ok(request) => request,
            Err(error) => {
                let reason = error.to_string();
                control.send(&Reply::Rejected(&reason).encode());
                continue;
            }
        };
        if let Some(status) = perform(&mut machine, control, manifest, request) {
            return status;
        }
    }
    exit::REQUEST_BUDGET
}

/// Performs one request, replying exactly once, and returns the exit status when it ends the
/// session.
fn perform(
    machine: &mut Machine,
    control: &Channel,
    manifest: &DescriptorManifest,
    request: Request,
) -> Option<i32> {
    match request {
        Request::Attest => control.send(&attest(manifest).encode()),
        Request::Launch(launch) => match machine.launch(launch) {
            Ok(ready) => control.send(&Reply::Ready(&ready).encode()),
            Err(failure) => control.send(&Reply::Failure(&failure).encode()),
        },
        Request::Execute(execute) => match machine.execute(execute) {
            Ok(executed) => control.send(&Reply::Executed(&executed).encode()),
            Err(failure) => control.send(&Reply::Failure(&failure).encode()),
        },
        Request::Stop(stop) => match machine.stop(stop) {
            Ok(stopped) => {
                control.send(&Reply::Stopped(&stopped).encode());
                // The Machine proved its cleanup, so this worker has nothing left to own.
                return Some(exit::OK);
            }
            Err(failure) => control.send(&Reply::Failure(&failure).encode()),
        },
        Request::Output(window) => {
            let bytes = machine.output(&window).unwrap_or_default();
            control.send(&Reply::Output(bytes).encode());
        }
        Request::Seal => control.send(&seal().encode()),
        Request::Shutdown(status) => return Some(status),
    }
    None
}

/// Narrows the filter to its steady-state phase, after which the startup-only syscalls that
/// attestation and setup need are seccomp kills rather than errors.
fn seal<'a>() -> Reply<'a> {
    match install_filter(Phase::SteadyState) {
        Ok(()) => Reply::Sealed,
        // A failed transition leaves the startup filter in place; the kernel installs a filter
        // atomically, so nothing partial can be running.
        Err(_) => Reply::Rejected("the steady-state filter could not be installed"),
    }
}
