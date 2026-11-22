//! The thread entry point: one boot, one machine, and the path it takes to Ready.

use std::fs::File;
use std::sync::mpsc::{Receiver, Sender};

use soma_guest::HostLaunchMaterial;
use soma_kvm::x86_64::{RestoreRequest, SandboxMachine, restore};

use super::super::session::{Request, Response, SessionError};
use super::super::source::{Boot, Network, Source};
use super::drive::{drive_cold, drive_restored};
use super::finish::{Ending, report};
use super::inputs::LaunchInputs;
use super::suspend::{self, SuspendDisks};

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
            report(sandbox, &outcome, responses, instance);
        }
        Source::Restore {
            objects,
            hypervisor,
            disks,
            volume,
            devices,
            memory_bytes,
            vcpus,
            contract,
            hotplug,
        } => {
            // The volume is opened from the path the store certified, and a file that cannot be
            // opened refuses the launch rather than serving an empty disk.
            let data = match volume
                .as_ref()
                .map(|attachment| super::super::volume::open(&attachment.path))
            {
                Some(Ok(file)) => Some(file),
                Some(Err(_)) => {
                    let _ignored = responses.send(Response::Failed(SessionError::Create));
                    return;
                }
                None => None,
            };
            // A head is a fresh inode: its pages are shared with the Generation's template but
            // not resident in it, so the guest's first read faults every page it touches in.
            // Recording which ranges those were is what lets the next head for this Generation be
            // warmed while it is prepared, instead of while a guest waits.
            let reads = soma_kvm::head_reads::recording();
            // A suspension reads the root's identity and copies the private head, both of which
            // the restore takes ownership of; duplicating the two descriptors now costs nothing
            // and is the only way to reach the head, which is already unlinked.
            let Ok(suspend_disks) = duplicate(&disks) else {
                let _ignored = responses.send(Response::Failed(SessionError::Create));
                return;
            };
            let restored = restore(RestoreRequest {
                objects,
                hypervisor,
                disks,
                volume: data,
                devices,
                guest_cid,
                memory_bytes,
                vcpus,
                contract,
                // Re-hashing every byte of the memory object is the installation and audit
                // boundary, not the request path.
                verify_artifacts: false,
                hotplug,
                // An Instance the broker leased a bundle to arrives here with its frame path;
                // one that asked for no egress keeps the device it was built with, whose link
                // stays down and which drops every frame.
                network: attachment,
                reads: Some(std::sync::Arc::clone(&reads)),
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
                &reads,
            );
            if let Ok(Ending::Suspended(target)) = &outcome {
                let read = (suspend_disks, generation, contract);
                answer_suspension(&mut restored.machine, target, read, responses);
            }
            report(restored.machine, &outcome, responses, instance);
        }
    }
}

/// Duplicates the two descriptors a suspension reads before the restore takes the originals.
fn duplicate(disks: &soma_kvm::x86_64::SandboxDisks) -> std::io::Result<SuspendDisks> {
    Ok(SuspendDisks {
        root: disks.root.try_clone()?,
        overlay: disks.overlay.as_ref().map(File::try_clone).transpose()?,
    })
}

/// Reads a suspended machine into its target and answers the owner with the result.
fn answer_suspension(
    machine: &mut SandboxMachine,
    target: &super::super::session::SuspendTarget,
    read: (SuspendDisks, [u8; 32], soma_kvm::contract::MachineContract),
    responses: &Sender<Response>,
) {
    let (mut disks, generation, contract) = read;
    let answer = match suspend::read(machine, &mut disks, target, generation, contract) {
        Ok(captured) => Response::Suspended(Box::new(captured)),
        Err(error) => Response::Failed(error),
    };
    let _ignored = responses.send(answer);
}
