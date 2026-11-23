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

#[cfg(test)]
mod tests {
    use std::{
        fs::TryLockError,
        time::{Duration, Instant},
    };

    use super::super::testing::spinning_machine;
    use crate::virtio::{GuestAddress, GuestMemory as _};
    use crate::x86_64::{EXPECTED_SERIAL, GuestExit};

    #[test]
    #[ignore = "requires a Linux x86_64 host with accessible /dev/kvm"]
    fn a_paused_machine_rebinds_its_volume_without_losing_guest_memory() {
        const GRACE: Duration = Duration::from_secs(2);
        let original = tempfile::NamedTempFile::new().unwrap();
        let original_handle = original.reopen().unwrap();
        original_handle.set_len(4096).unwrap();
        original_handle.try_lock().unwrap();
        let probe = original.reopen().unwrap();
        let next = tempfile::NamedTempFile::new().unwrap();
        let next_handle = next.reopen().unwrap();
        next_handle.set_len(4096).unwrap();
        next_handle.try_lock().unwrap();

        let (mut machine, address) = spinning_machine(Some(original_handle));
        assert!(machine.set_paused_workspace(None).is_err());
        machine.watch_console(EXPECTED_SERIAL);
        machine.start().unwrap();
        machine.wait_console_line(Instant::now() + GRACE).unwrap();
        machine.pause(GRACE).unwrap();
        assert!(matches!(probe.try_lock(), Err(TryLockError::WouldBlock)));
        let counter = |machine: &super::SandboxMachine| {
            machine
                .with_paused(|paused| {
                    let mut value = [0; 4];
                    paused
                        .memory
                        .read_bytes(GuestAddress(u64::from(address)), &mut value)
                        .unwrap();
                    value
                })
                .unwrap()
        };
        let before = counter(&machine);
        machine.set_paused_workspace(None).unwrap();
        probe.try_lock().unwrap();
        assert_eq!(counter(&machine), before);
        machine.set_paused_workspace(Some(next_handle)).unwrap();
        machine.resume().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        machine.pause(GRACE).unwrap();
        assert!(u32::from_le_bytes(counter(&machine)) > u32::from_le_bytes(before));
        assert_eq!(machine.finish(Duration::ZERO).exit, Ok(GuestExit::Paused));
    }
}
