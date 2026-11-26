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
    sandbox::{Boot, Session, SessionError, Source, guest_cid_for},
};

use super::{KvmPlatform, MachineResources, faults, network_for_cid};

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
        if !resources.volume_matches_capacity(declared.volume_capacity())
            || (resources.network.is_some() && !declared.network())
        {
            return Err(invalid());
        }
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
            .with_volume(declared.volume_capacity().is_some())
            .with_mem(hotplug.is_some());
        let generation = *launch.generation().id().as_bytes();
        let instance = *launch.instance_id().as_bytes();
        let operation = *launch.operation_id().as_bytes();
        let guest_cid = guest_cid_for(instance);
        let (network, activation) = network_for_cid(resources.network, guest_cid)
            .map_err(|error| RestoreFailure::at_restore(faults::restore_recovery(error)))?;
        let request = ResidentRestoreRequest {
            objects: SnapshotObjects::adopt(
                resources.state,
                resources.memory,
                selection.saved_overlay,
            ),
            hypervisor: Hypervisor::Adopted(resources.kvm),
            root: resources.root,
            head: resources.overlay,
            volume: resources.volume,
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
                network,
                secrets: Vec::new(),
            },
            &mut |receipt| {
                activation
                    .as_ref()
                    .ok_or(SessionError::Network)?
                    .ask(receipt)
            },
        )
        .map_err(|error| match error {
            SessionError::CheckpointIncompatible => RestoreFailure::checkpoint_incompatible(),
            other => RestoreFailure::at_restore(faults::restore_recovery(other)),
        })?;
        self.session = Some(session);
        self.recovered = true;
        self.checkpoint_geometry = Some((
            generation,
            contract,
            declared.writable_disk(),
            declared.volume_capacity().is_some(),
        ));
        RestoredMachine::from_observation(RestoreProgress::from_steps([
            RestoreStep::ArtifactsVerified,
            RestoreStep::MachineRestored,
        ]))
    }
}
