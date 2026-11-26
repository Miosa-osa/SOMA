//! The thread entry point: one boot, one machine, and the path it takes to Ready.

use std::fs::File;
use std::sync::mpsc::{Receiver, Sender};

use soma_guest::HostLaunchMaterial;
use soma_kvm::x86_64::{NetworkAttachment, RestoreRequest, SandboxConfig, SandboxMachine, restore};

use super::super::pending::PendingActivation;
use super::super::session::{Request, Response, SessionError};
use super::super::source::{Boot, Source};
use super::drive::{drive_checkpoint, drive_cold, drive_restored};
use super::finish::{Ending, report};
use super::inputs::LaunchInputs;
use super::suspend::{self, SuspendDisks};

mod recovery;

use recovery::{matches_recovery_source, recover};

/// Owns one machine for its whole life and answers requests about it.
pub fn serve(boot: Boot, requests: &Receiver<Request>, responses: &Sender<Response>) {
    let Some(material) = admitted_material(&boot) else {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    };
    let Boot {
        source,
        generation,
        instance,
        operation,
        guest_cid,
        network,
        secrets,
    } = boot;
    let attachment = network.attachment;
    let activation = network.activation;

    match source {
        Source::ResidentCheckpoint(request) | Source::CheckpointFork(request) => {
            recover(
                *request,
                material,
                (instance, operation),
                (attachment, activation),
                requests,
                responses,
            );
        }
        Source::ColdBoot(config) => {
            serve_cold(
                config,
                LaunchInputs {
                    material,
                    secrets: &secrets,
                    suspendable: false,
                },
                (instance, attachment, activation),
                requests,
                responses,
            );
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
            let Ok(data) = open_volume(volume.as_ref()) else {
                let _ignored = responses.send(Response::Failed(SessionError::Create));
                return;
            };
            // A head is a fresh inode: its pages are shared with the Generation's template but
            // not resident in it, so the guest's first read faults every page it touches in.
            // Recording which ranges those were is what lets the next head for this Generation be
            // warmed while it is prepared, instead of while a guest waits.
            let reads = soma_kvm::head_reads::recording();
            // A suspension reads the root and the already unlinked head, which the restore takes.
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
                suspendable: true,
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

fn admitted_material(boot: &Boot) -> Option<HostLaunchMaterial> {
    if !matches_recovery_source(boot) {
        return None;
    }
    HostLaunchMaterial::generate(
        boot.generation,
        boot.instance,
        boot.operation,
        boot.network.launch,
    )
    .ok()
}

fn serve_cold(
    config: SandboxConfig,
    inputs: LaunchInputs<'_>,
    network: (
        [u8; 16],
        Option<NetworkAttachment>,
        Option<PendingActivation>,
    ),
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) {
    let (instance, attachment, activation) = network;
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
    let outcome = drive_cold(&mut sandbox, inputs, activation, requests, responses);
    report(sandbox, &outcome, responses, instance);
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

/// Opens the certified volume, if the Instance has one; a file that cannot be opened is an error.
fn open_volume(volume: Option<&soma::VolumeAttachment>) -> std::io::Result<Option<File>> {
    volume
        .map(|attachment| super::super::volume::open(&attachment.path))
        .transpose()
}

#[cfg(test)]
mod tests {
    use crate::sandbox::source::Network;

    use super::{Boot, Source, matches_recovery_source};
    use soma_guest::LaunchNetwork;
    use soma_kvm::{
        DeviceSet, MachineContract,
        snapshot::Digest,
        x86_64::{Hypervisor, ResidentRestoreRequest, SnapshotObjects},
    };

    fn recovery_boot() -> Boot {
        let file = || tempfile::tempfile().expect("test file");
        Boot {
            source: Source::ResidentCheckpoint(Box::new(ResidentRestoreRequest {
                objects: SnapshotObjects::adopt(file(), file(), None),
                hypervisor: Hypervisor::Device,
                root: file(),
                head: None,
                volume: None,
                instance_id: [1; 16],
                candidate_id: [2; 32],
                state_digest: Digest::of(b"state"),
                devices: DeviceSet::new(false, false),
                guest_cid: 4,
                memory_bytes: 1 << 30,
                vcpus: 1,
                contract: MachineContract::V1,
                hotplug: None,
                attestations: soma_kvm::snapshot::RestoreAttestations::default(),
            })),
            instance: [1; 16],
            generation: [2; 32],
            operation: [3; 16],
            guest_cid: 4,
            network: Network {
                launch: LaunchNetwork::new(
                    4,
                    4,
                    [2, 0, 0, 0, 0, 1],
                    [10, 0, 0, 2],
                    24,
                    [10, 0, 0, 1],
                    [10, 0, 0, 1],
                    1,
                )
                .expect("link-down test network"),
                attachment: None,
                activation: None,
            },
            secrets: Vec::new(),
        }
    }

    #[test]
    fn a_checkpoint_worker_refuses_mismatched_launch_authority_before_kvm() {
        let mut boot = recovery_boot();
        assert!(matches_recovery_source(&boot));
        boot.instance = [4; 16];
        assert!(!matches_recovery_source(&boot));
        boot.instance = [1; 16];
        boot.generation = [4; 32];
        assert!(!matches_recovery_source(&boot));
        boot.generation = [2; 32];
        boot.guest_cid = 5;
        assert!(!matches_recovery_source(&boot));
    }
    #[test]
    fn a_checkpoint_fork_requires_a_distinct_destination_and_no_new_authority() {
        let mut boot = recovery_boot();
        let Source::ResidentCheckpoint(request) = boot.source else {
            unreachable!();
        };
        boot.source = Source::CheckpointFork(request);
        assert!(!matches_recovery_source(&boot));
        boot.instance = [5; 16];
        assert!(matches_recovery_source(&boot));
        boot.guest_cid = 5;
        assert!(!matches_recovery_source(&boot));
    }
}
