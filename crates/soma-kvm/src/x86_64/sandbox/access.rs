//! Hotplug, control channel, milestones and command line of a created machine.

use super::{Arc, ControlChannel, MachineError, Milestone, Phase, PoisonError, SandboxMachine};

impl SandboxMachine {
    /// Adds the empty hotplug region behind its own KVM memory slot and returns the backing the
    /// memory device hands unplugged blocks to.
    ///
    /// Call it before [`Self::start`]. Boot RAM, the memory object, and the certified slot list
    /// are untouched; the region is neither captured nor restored as memory.
    ///
    /// # Errors
    ///
    /// Fails when the machine already has a region or the host refuses the mapping or slot.
    pub fn attach_hotplug(
        &mut self,
        region: crate::memory_layout::HotplugRegion,
    ) -> Result<crate::x86_64::HotplugBacking, MachineError> {
        if self.machine.ram.hotplug().is_some() {
            return Err(MachineError::invalid(
                Phase::MapMemory,
                "the machine already has a hotplug region",
            ));
        }
        self.machine.ram.attach_hotplug(region)?;
        let ram = self.machine.ram.hotplug().ok_or_else(|| {
            MachineError::invalid(Phase::MapMemory, "the hotplug region was not attached")
        })?;
        ram.register(&self.machine.vm)?;
        Ok(crate::x86_64::HotplugBacking::new(ram))
    }

    /// The backing over the machine's hotplug region, for a machine built with one or restored
    /// from a snapshot that carried one. Every call shares the same mapping, so the memory
    /// device can be rebuilt from it after a restore.
    #[must_use]
    pub fn hotplug_backing(&self) -> Option<crate::x86_64::HotplugBacking> {
        self.machine.ram.hotplug_backing()
    }

    /// The hotplug region, when the machine has one.
    #[must_use]
    pub fn hotplug_region(&self) -> Option<crate::memory_layout::HotplugRegion> {
        self.machine.ram.hotplug().map(|ram| ram.region())
    }

    /// The byte channel over the guest's control connection.
    #[must_use]
    pub fn control(&self) -> ControlChannel {
        ControlChannel::new(
            Arc::clone(&self.shared),
            Arc::clone(&self.host_work),
            Arc::clone(&self.finished),
        )
    }

    /// Records a caller-observed milestone.
    pub fn mark(&self, milestone: Milestone) {
        self.timeline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .mark(milestone);
    }

    /// The exact command line written to the guest.
    #[must_use]
    pub fn cmdline(&self) -> &str {
        &self.cmdline
    }
}
