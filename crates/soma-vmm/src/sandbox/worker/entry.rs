//! The thread entry point: one boot, one machine, and the path it takes to Ready.

use std::sync::mpsc::{Receiver, Sender};

use soma_guest::HostLaunchMaterial;
use soma_kvm::x86_64::{RestoreRequest, SandboxMachine, restore};

use super::super::session::{Request, Response, SessionError};
use super::super::source::{Boot, Network, Source};
use super::drive::{drive_cold, drive_restored};
use super::finish::report;
use super::inputs::LaunchInputs;

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
        } => {
            let restored = restore(RestoreRequest {
                objects,
                hypervisor,
                disks,
                devices,
                guest_cid,
                memory_bytes,
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
