use std::fs::File;

use soma_kvm::{
    DeviceSet, MachineContract,
    memory_layout::{GuestLayout, HotplugRegion},
    snapshot::Digest,
    x86_64::{Hypervisor, ResidentRestoreRequest, SnapshotObjects},
};

use crate::{
    Launch, Recovery,
    platform::{RestoreFailure, RestoreProgress, RestoreStep, RestoredMachine},
    sandbox::{Boot, Network, Session, Source, guest_cid_for, link_down_network},
};

use super::{KvmPlatform, MachineResources, faults};

pub(super) struct Selection {
    pub(super) digest: Digest,
    pub(super) saved_overlay: Option<File>,
    pub(super) fork_from: Option<crate::InstanceId>,
}

impl KvmPlatform {
    pub(super) fn restore_checkpoint(
        &mut self,
        launch: &Launch,
        resources: MachineResources,
        selection: Selection,
    ) -> Result<RestoredMachine, RestoreFailure> {
        let invalid = || RestoreFailure::at_restore(Recovery::DoNotRetry);
        let spec = launch.generation().machine();
        let declared = launch.generation().devices();
        let contract = MachineContract::require(spec.contract().get()).map_err(|_| invalid())?;
        let memory_bytes = spec.memory().get();
        let hotplug = spec
            .hotplug()
            .map(|bytes| {
                GuestLayout::new(memory_bytes)
                    .ok()
                    .and_then(|layout| HotplugRegion::new(layout, bytes.get()).ok())
                    .ok_or_else(invalid)
            })
            .transpose()?;
        let devices = DeviceSet::new(declared.writable_disk(), declared.network())
            .with_mem(hotplug.is_some());
        let generation = *launch.generation().id().as_bytes();
        let instance = *launch.instance_id().as_bytes();
        let operation = *launch.operation_id().as_bytes();
        let guest_cid = guest_cid_for(instance);
        let network = link_down_network(guest_cid).ok_or_else(invalid)?;
        let request = ResidentRestoreRequest {
            objects: SnapshotObjects::adopt(
                resources.state,
                resources.memory,
                selection.saved_overlay,
            ),
            hypervisor: Hypervisor::Adopted(resources.kvm),
            root: resources.root,
            head: resources.overlay,
            instance_id: selection.fork_from.map_or(instance, |id| *id.as_bytes()),
            candidate_id: generation,
            state_digest: selection.digest,
            devices,
            guest_cid,
            memory_bytes,
            vcpus: spec.vcpus().get(),
            contract,
            hotplug,
        };
        let source = if selection.fork_from.is_some() {
            Source::CheckpointFork(Box::new(request))
        } else {
            Source::ResidentCheckpoint(Box::new(request))
        };
        let session = Session::launch(
            Boot {
                source,
                generation,
                instance,
                operation,
                guest_cid,
                network: Network {
                    launch: network,
                    attachment: None,
                    activation: None,
                },
                secrets: Vec::new(),
            },
            &mut |_| Err(crate::sandbox::SessionError::Network),
        )
        .map_err(|error| RestoreFailure::at_restore(faults::restore_recovery(error)))?;
        self.session = Some(session);
        self.recovered = true;
        self.checkpoint_geometry = Some((generation, contract, declared.writable_disk()));
        RestoredMachine::from_observation(RestoreProgress::from_steps([
            RestoreStep::ArtifactsVerified,
            RestoreStep::MachineRestored,
        ]))
    }
}
