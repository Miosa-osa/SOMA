//! The thread entry point: one boot, one machine, and the path it takes to Ready.

use std::sync::mpsc::{Receiver, Sender};

use soma_guest::HostLaunchMaterial;
use soma_kvm::x86_64::{
    NetworkAttachment, RestoreRequest, SandboxMachine, prepare_checkpoint, prepare_fork, restore,
};

use super::super::pending::PendingActivation;
use super::super::session::{Request, Response, SessionError};
use super::super::source::{Boot, Network, Source};
use super::drive::{drive_checkpoint, drive_cold, drive_restored};
use super::finish::report;
use super::inputs::LaunchInputs;

/// Owns one machine for its whole life and answers requests about it.
pub fn serve(boot: Boot, requests: &Receiver<Request>, responses: &Sender<Response>) {
    if !matches_recovery_source(&boot) {
        let _ignored = responses.send(Response::Failed(SessionError::Create));
        return;
    }
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
        Source::ResidentCheckpoint(request) | Source::CheckpointFork(request) => {
            recover(
                *request,
                material,
                instance,
                operation,
                (attachment, activation),
                requests,
                responses,
            );
        }
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
            report(restored.machine, outcome, responses, instance);
        }
    }
}

fn matches_recovery_source(boot: &Boot) -> bool {
    let (request, names_expected_instance, admit_network) = match &boot.source {
        Source::ResidentCheckpoint(request) => (
            request,
            request.instance_id == boot.instance,
            request.devices.net(),
        ),
        Source::CheckpointFork(request) => (request, request.instance_id != boot.instance, false),
        _ => return true,
    };
    let network_valid = match (&boot.network.attachment, &boot.network.activation) {
        (None, None) => true,
        (Some(attachment), Some(_)) => admit_network && attachment.mac == boot.network.launch.mac(),
        _ => false,
    };
    network_valid
        && boot.secrets.is_empty()
        && names_expected_instance
        && request.candidate_id == boot.generation
        && request.guest_cid == boot.guest_cid
        && boot.network.launch.vsock_cid() == boot.guest_cid
}

fn recover(
    request: soma_kvm::x86_64::ResidentRestoreRequest,
    material: HostLaunchMaterial,
    instance: [u8; 16],
    operation: [u8; 16],
    network: (Option<NetworkAttachment>, Option<PendingActivation>),
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) {
    let source = request.instance_id;
    let (prepared, material) = if source == instance {
        (prepare_checkpoint(request), Ok(material))
    } else {
        let authority = material.for_fork(source);
        (prepare_fork(request, instance), authority)
    };
    let Ok(material) = material else {
        let _ignored = responses.send(Response::Failed(SessionError::LaunchPage));
        return;
    };
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("soma-vmm: checkpoint validation failed: {error}");
            let _ignored = responses.send(Response::Failed(SessionError::Create));
            return;
        }
    };
    let (attachment, activation) = network;
    if let Some(attachment) = attachment {
        prepared.attach_network(attachment);
    }
    let mut restored = None;
    let delivered = material.deliver_with(|page| {
        restored = Some(prepared.resume(page)?);
        Ok::<(), soma_kvm::x86_64::SnapshotError>(())
    });
    let (Ok(delivered), Some(restored)) = (delivered, restored) else {
        let _ignored = responses.send(Response::Failed(SessionError::LaunchPage));
        return;
    };
    let outcome = drive_checkpoint(
        &restored,
        delivered,
        (instance, operation, activation),
        requests,
        responses,
    );
    report(restored.machine, outcome, responses, instance);
}

#[cfg(test)]
mod tests {
    use super::{Boot, Network, Source, matches_recovery_source};
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
                instance_id: [1; 16],
                candidate_id: [2; 32],
                state_digest: Digest::of(b"state"),
                devices: DeviceSet::new(false, false),
                guest_cid: 4,
                memory_bytes: 1 << 30,
                vcpus: 1,
                contract: MachineContract::V1,
                hotplug: None,
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
