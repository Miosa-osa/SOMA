//! The jailed SOMA VMM worker.
//!
//! The `soma-jail` launcher builds the jail and `execveat`s this binary inside it with the
//! descriptor manifest as its only argument and an empty environment.
//! There is no filesystem to read and no path to open, so every resource the worker has is a
//! descriptor the launcher sealed into a manifest slot before the exec.
//!
//! The worker therefore starts by proving where it is: it attests the sealed descriptor table,
//! its ephemeral identity, and its empty root, sends that attestation over the control
//! descriptor, and refuses to serve anything at all when the attestation does not describe a
//! jail. Once it is admitted it serves the lifecycle contract of this crate, one bounded
//! request packet at a time, until it is told to stop.
//!
//! This binary does not restore a machine. It is the process the jail constrains and the
//! contract endpoint the supervisor talks to; machine restoration arrives behind the same
//! [`Machine`](soma_vmm::Machine) it already drives.
//!
//! It is only meaningful on Linux `x86_64` inside a jail; elsewhere it exits with
//! [`exit::UNSUPPORTED`].

#![deny(warnings)]

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod admission;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod channel;
// The status contract is stated in full on every target, though no single target can reach
// every value: `UNSUPPORTED` exists only off the production target and the rest only on it.
#[allow(dead_code)]
mod exit;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod session;

/// Attests containment, reports it, and serves the lifecycle when the attestation admits it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run() -> i32 {
    use soma_jail::{DescriptorManifest, DescriptorRole, attest};

    let Some(encoded) = std::env::args().nth(1) else {
        return exit::NO_MANIFEST;
    };
    let Ok(manifest) = DescriptorManifest::decode(&encoded) else {
        return exit::NO_MANIFEST;
    };
    let Some(control) = manifest
        .slot_for(DescriptorRole::Control)
        .and_then(channel::Channel::open)
    else {
        return exit::NO_CONTROL;
    };
    let attestation = attest(&manifest);
    // The attestation is sent even when it refuses service, because it is the only way the
    // supervisor can learn which property of the jail was not met.
    control.send(&attestation.encode());
    if admission::admits_service(&attestation) {
        // This process holds one machine, so a guest that ends it from inside ends the process,
        // and the exit status tells the supervisor whether the guest shut down or restarted.
        soma_vmm::sandbox::guest_end::exit_process_on_guest_end();
        debug_assert_eq!(
            soma_vmm::sandbox::guest_end::GUEST_POWER_OFF_STATUS,
            exit::GUEST_POWER_OFF
        );
        debug_assert_eq!(
            soma_vmm::sandbox::guest_end::GUEST_REBOOT_STATUS,
            exit::GUEST_REBOOT
        );
        session::serve(&control, &manifest)
    } else {
        exit::UNCONTAINED
    }
}

fn main() {
    // The launcher passes one encoded descriptor manifest, never this word, so a version request
    // can only come from an operator or the release packager.
    if soma_version::version_requested(std::env::args_os()) {
        let protocols = [soma_version::Protocol::new(
            "vmm-control",
            u32::from(soma_vmm::control::CONTROL_PROTOCOL_VERSION),
        )];
        let speaks = soma_version::Speaks {
            api: None,
            protocols: &protocols,
        };
        print!(
            "{}",
            soma_version::report("soma-vmm", &soma_version::build!(), &speaks)
        );
        return;
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    std::process::exit(run());
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    std::process::exit(exit::UNSUPPORTED);
}
