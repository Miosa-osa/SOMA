//! Capturing a machine that holds plugged memory: give it back, capture, plug it again.
//!
//! A checkpoint holds the memory a machine booted with and nothing of its memory device's
//! region, so a capture refuses while any block is plugged. This runs inside the one host call
//! that captures, so no other resize can come between the steps: unplug to the boot size,
//! capture, then plug the same total again whatever the capture answered. A guest that cannot
//! give its memory back is refused the capture rather than captured with memory it would lose.
//! A restore grows the machine to the shape it is asked for, as every launch does.

use soma::InstanceId;

use super::{link::Link, memory};
use crate::{CheckpointFailure, backend::kvm::KvmBackend};

impl KvmBackend {
    /// Runs `capture` with the machine of `instance` at its boot memory, and grows it back after.
    pub(in crate::backend::kvm) fn capture_at_boot_memory<T>(
        &mut self,
        instance: &InstanceId,
        capture: impl FnOnce(&mut Self) -> Result<T, CheckpointFailure>,
    ) -> Result<T, CheckpointFailure> {
        let live = self.live_for(instance).ok_or(CheckpointFailure::NotFound)?;
        let booted = live.memory_mib;
        // A paused guest cannot move its memory, and its capture is refused on its own terms.
        let before = (!live.paused)
            .then(|| memory::current(&mut Link::of(&mut live.held, instance), booted))
            .filter(|level| level.plugged_mb > 0);
        let Some(before) = before else {
            return capture(self);
        };
        let given_back = memory::resize(
            &mut Link::of(&mut live.held, instance),
            booted,
            booted,
            &memory::REAL,
            &mut std::thread::sleep,
        );
        let result = if given_back.plugged_mb == 0 {
            capture(self)
        } else {
            eprintln!(
                "soma: checkpoint of {} refused: the guest kept {} MiB of plugged memory",
                instance.as_str(),
                given_back.plugged_mb
            );
            Err(CheckpointFailure::CaptureRefused)
        };
        if let Some(live) = self.live_for(instance) {
            let regrown = memory::resize(
                &mut Link::of(&mut live.held, instance),
                booted,
                before.total_mb,
                &memory::REAL,
                &mut std::thread::sleep,
            );
            if regrown.total_mb != before.total_mb {
                eprintln!(
                    "soma: {} has {} MiB after its checkpoint, not the {} MiB it had before",
                    instance.as_str(),
                    regrown.total_mb,
                    before.total_mb
                );
            }
        }
        result
    }
}
