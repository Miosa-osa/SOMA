//! Restore a machine from its Generation's verified artifacts when no checkpoint was selected.

use super::{
    DeviceSet, GuestLayout, HotplugRegion, Hypervisor, KvmPlatform, Launch, MachineContract,
    MachineResources, Recovery, RestoreFailure, RestoreProgress, RestoreStep, RestoredMachine,
    Session, SessionError, SnapshotObjects, SterileSpec, faults,
};

impl KvmPlatform {
    /// Verifies the declared devices and shape against the adopted resources, then prepares the
    /// sterile session that restores this Generation.
    pub(super) fn restore_generation(
        &mut self,
        launch: &Launch,
    ) -> Result<RestoredMachine, RestoreFailure> {
        let Some(resources) = self.resources.take() else {
            return Err(RestoreFailure::at_restore(Recovery::DoNotRetry));
        };
        if let Some(selection) = self.recovery.take() {
            return self.restore_checkpoint(launch, resources, selection);
        }
        let declared = launch.generation().devices();
        if resources.network.is_some() && !declared.network() {
            return Err(RestoreFailure::at_restore(Recovery::DoNotRetry));
        }
        let volume_capacity_bytes = declared.volume_capacity().map(std::num::NonZeroU64::get);
        if !resources.volume_matches_capacity(declared.volume_capacity()) {
            return Err(RestoreFailure::at_restore(Recovery::DoNotRetry));
        }
        let overlay_capacity_bytes = declared
            .writable_disk()
            .then(|| resources.overlay_capacity_bytes())
            .flatten();
        let MachineResources {
            kvm,
            state,
            memory,
            root,
            overlay,
            volume,
            network,
        } = resources;
        self.overlay = overlay;
        self.volume = volume;
        let machine = launch.generation().machine();
        // The contract arrives as the portable version the parent stated, and a version this
        // build does not implement is refused here rather than restored as the wrong machine.
        let shape = MachineContract::require(machine.contract().get())
            .ok()
            .zip(Some(machine.vcpus().get()));
        let Some((contract, vcpus)) = shape else {
            // A version this build does not implement, or a shape that does not fit the
            // contract's own field widths, is a property of this host and its store rather
            // than of the request, so it lands where the other restore faults do.
            return Err(RestoreFailure::at_restore(faults::restore_recovery(
                SessionError::Create,
            )));
        };
        // The region is placed above this machine's RAM from the declared size alone, so a restore
        // and a cold boot of the same Generation put it at the same address.
        let hotplug = match machine.hotplug() {
            None => None,
            Some(bytes) => {
                let placed = GuestLayout::new(machine.memory().get())
                    .ok()
                    .and_then(|layout| HotplugRegion::new(layout, bytes.get()).ok());
                let Some(region) = placed else {
                    return Err(RestoreFailure::at_restore(faults::restore_recovery(
                        SessionError::Create,
                    )));
                };
                Some(region)
            }
        };
        let spec = SterileSpec {
            objects: SnapshotObjects::adopt(state, memory, None),
            hypervisor: Hypervisor::Adopted(kvm),
            root,
            overlay_capacity_bytes,
            memory_bytes: machine.memory().get(),
            vcpus,
            contract,
            devices: DeviceSet::new(declared.writable_disk(), declared.network())
                .with_mem(hotplug.is_some())
                .with_volume(volume_capacity_bytes.is_some()),
            hotplug,
            volume_capacity_bytes,
        };
        match Session::prepare(spec) {
            Ok(session) => {
                self.session = Some(session);
                self.network = network;
                self.checkpoint_geometry = Some((
                    *launch.generation().id().as_bytes(),
                    contract,
                    declared.writable_disk(),
                    volume_capacity_bytes.is_some(),
                ));
                RestoredMachine::from_observation(RestoreProgress::from_steps([
                    RestoreStep::ArtifactsVerified,
                    RestoreStep::MachineRestored,
                ]))
            }
            // Nothing is re-hashed on this path, so every failure here is in the restore rather
            // than in a verification that never ran.
            Err(error) => Err(RestoreFailure::at_restore(faults::restore_recovery(error))),
        }
    }
}
