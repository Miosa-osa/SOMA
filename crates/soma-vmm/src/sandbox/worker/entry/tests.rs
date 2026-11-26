use crate::sandbox::{PendingActivation, source::Network};

use super::{Boot, Source, matches_recovery_source};
use soma_guest::{ActivationChallenge, LaunchNetwork};
use soma_kvm::{
    DeviceSet, MachineContract,
    snapshot::Digest,
    x86_64::{Hypervisor, NetworkAttachment, ResidentRestoreRequest, SnapshotObjects},
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

fn networked_fork() -> Boot {
    let mut boot = recovery_boot();
    let Source::ResidentCheckpoint(mut request) = boot.source else {
        unreachable!();
    };
    request.devices = DeviceSet::new(false, true);
    boot.source = Source::CheckpointFork(request);
    boot.instance = [5; 16];
    boot.network.attachment = Some(NetworkAttachment {
        tap: tempfile::tempfile().expect("tap stand-in"),
        mac: boot.network.launch.mac(),
    });
    boot.network.activation = Some(PendingActivation {
        challenge: ActivationChallenge::from_bytes([9; 32]).expect("challenge"),
        generation: 1,
        intent: [7; 32],
    });
    boot
}

#[test]
fn a_checkpoint_fork_takes_its_own_network_on_a_network_device() {
    let mut boot = networked_fork();
    assert!(matches_recovery_source(&boot));
    // The attachment must carry the MAC the child's own launch page names.
    boot.network.attachment.as_mut().expect("attachment").mac = [2, 0, 0, 0, 0, 9];
    assert!(!matches_recovery_source(&boot));

    let mut boot = networked_fork();
    let Source::CheckpointFork(request) = &mut boot.source else {
        unreachable!();
    };
    // A checkpoint captured with no network device cannot be given one.
    request.devices = DeviceSet::new(false, false);
    assert!(!matches_recovery_source(&boot));

    let mut boot = networked_fork();
    boot.network.activation = None;
    assert!(!matches_recovery_source(&boot));
}
