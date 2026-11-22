//! Rebinding the workspace block backend while its guest is quiescent.

use std::fs::File;

use crate::{
    virtio::FileBackend,
    x86_64::error::{MachineError, MachineErrorKind, Phase},
};

use super::SandboxMachine;

impl SandboxMachine {
    /// Changes the backing file of a declared workspace slot while the machine is paused.
    ///
    /// The broker must first confirm the guest unmounted its workspace.
    /// A new backing file must already hold the broker's exclusive volume lock.
    /// The slot, device capacity, and queue state stay in place when detaching.
    ///
    /// # Errors
    ///
    /// Refuses a running machine, a missing slot, pending disk work or an incompatible backing.
    /// If flushing a departing volume fails, it remains attached.
    pub fn set_paused_workspace(&self, file: Option<File>) -> Result<(), MachineError> {
        let backing = file
            .map(|file| FileBackend::new(file, false))
            .transpose()
            .map_err(|error| MachineError::io(Phase::Devices, &error))?;
        self.with_paused(|mut paused| {
            let (queue, device) = paused
                .bus
                .volume_mut()
                .and_then(|slot| slot.queue_and_device_mut(0))
                .ok_or_else(|| workspace_error("the sandbox has no workspace volume slot"))?;
            if queue.is_ready()
                && queue
                    .pending(&paused.memory)
                    .map_err(|_| workspace_error("the volume queue could not be checked"))?
                    != 0
            {
                return Err(workspace_error("the volume queue still has pending I/O"));
            }
            match backing {
                Some(backing) => device.attach(Box::new(backing)).map_err(|error| {
                    MachineError::new(Phase::Devices, MachineErrorKind::Block(error))
                }),
                None => device
                    .release_volume()
                    .map_err(|_| workspace_error("the volume could not be flushed")),
            }
        })
        .ok_or_else(|| workspace_error("the sandbox is not paused"))?
    }
}

fn workspace_error(message: &'static str) -> MachineError {
    MachineError::invalid(Phase::Devices, message)
}
