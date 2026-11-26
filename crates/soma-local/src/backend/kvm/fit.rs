//! Whether a Generation can give a request, whatever process serves it.
//!
//! A shape the Generation cannot give reaches the caller as `generation_does_not_fit`, and a volume
//! it cannot take as `workload_rejected`. Neither carries the detail, so the reason is written
//! here, on the log of the process that decided it, naming the field, what was asked and what the
//! Generation gives.

use soma::{BackendFailureKind, InstanceId, MachineShape, VolumeAttachment};

use super::{KvmBackend, hotplug, prepared::PreparedGeneration};

const MIB: u64 = 1024 * 1024;

pub(super) fn boot_mib_of(prepared: &PreparedGeneration) -> u64 {
    prepared.manifest.shape.memory_bytes / MIB
}

/// Whether the Generation can give the request, whatever process serves it.
///
/// A host that jails its machines is stricter (memory must be exactly the boot size), and that
/// part is decided where the jail is known. The parent of a machine host asks this before it
/// claims a prepared machine, so a request that can never be served does not consume one or
/// count as demand.
pub(super) fn request_fits(prepared: &PreparedGeneration, shape: &MachineShape) -> bool {
    misfit(prepared, shape).is_none()
}

/// Why the Generation cannot give the request, or `None` when it can.
pub(super) fn misfit(prepared: &PreparedGeneration, shape: &MachineShape) -> Option<String> {
    let booted = Booted {
        vcpus: prepared.manifest.shape.vcpu_count,
        boot_mib: boot_mib_of(prepared),
        storage_bytes: prepared.manifest.template.writable_storage_bytes,
    };
    booted.misfit(shape, |total_mib| hotplug::room_for(prepared, total_mib))
}

/// What a Generation booted, as far as a request's shape is measured against it.
struct Booted {
    vcpus: u16,
    boot_mib: u64,
    storage_bytes: u64,
}

impl Booted {
    fn misfit(&self, shape: &MachineShape, room_for: impl Fn(u64) -> bool) -> Option<String> {
        let (asked, boot_mib) = (shape.memory_mib(), self.boot_mib);
        if shape.vcpu_count() < 1 || shape.vcpu_count() > self.vcpus {
            return Some(format!(
                "vcpu_count {} is outside 1..={}, the vCPUs the Generation booted",
                shape.vcpu_count(),
                self.vcpus
            ));
        }
        if asked < boot_mib {
            return Some(format!(
                "memory_mib {asked} is below the Generation's boot memory {boot_mib}"
            ));
        }
        if asked > boot_mib && !room_for(asked) {
            return Some(format!(
                "memory_mib {asked} is above the Generation's boot memory {boot_mib} and its \
                 memory device has no room for it"
            ));
        }
        let storage = self.storage_bytes;
        if !storage.is_multiple_of(MIB) || shape.storage_mib() != storage / MIB {
            return Some(format!(
                "storage_mib {} is not the Generation's writable storage of {storage} bytes",
                shape.storage_mib()
            ));
        }
        None
    }
}

/// Why the Generation cannot take the volume a fresh launch attaches, or `None` when it can.
///
/// The machine opens the volume again and decides for itself; this only names the reason on the
/// log of the process the caller talks to, which a machine host's own refusal never reaches.
pub(super) fn volume_misfit(
    prepared: &PreparedGeneration,
    volume: Option<&VolumeAttachment>,
) -> Option<String> {
    let persistence = &prepared.manifest.persistence;
    paired_volume_misfit(persistence.enabled, persistence.size_class_gib, volume)
}

fn paired_volume_misfit(
    enabled: bool,
    gib: u32,
    volume: Option<&VolumeAttachment>,
) -> Option<String> {
    match (enabled, volume) {
        (false, None) => None,
        (false, Some(_)) => Some(
            "a volume was attached but the Generation declares no persistent volume".to_owned(),
        ),
        (true, None) => Some(format!(
            "the Generation declares a {gib} GiB persistent volume and none was attached"
        )),
        (true, Some(volume)) => match std::fs::symlink_metadata(&volume.path) {
            Ok(info) if !info.is_file() => Some(format!(
                "the volume at {} is not a regular file",
                volume.path.display()
            )),
            Ok(info) if info.len() != u64::from(gib) << 30 => Some(format!(
                "the volume is {} bytes and the Generation's volume is {gib} GiB",
                info.len()
            )),
            // A volume that cannot be read is the host's fault, and the open reports it.
            _ => None,
        },
    }
}

/// Writes why a launch of `instance` is refused, and returns the kind the caller sees.
pub(super) fn refuse(instance: &InstanceId, reason: &str) -> BackendFailureKind {
    eprintln!(
        "soma: refused the launch of {}: workload_rejected: {reason}",
        instance.as_str()
    );
    BackendFailureKind::WorkloadRejected
}

/// Writes why the shape of a launch of `instance` does not fit its Generation, and returns the
/// kind of its own a placer can tell from every other refusal.
pub(super) fn refuse_shape(instance: &InstanceId, reason: &str) -> BackendFailureKind {
    eprintln!(
        "soma: refused the launch of {}: generation_does_not_fit: {reason}",
        instance.as_str()
    );
    BackendFailureKind::ShapeMisfit
}

impl KvmBackend {
    /// Why this Backend cannot give a request the vCPUs and memory it names.
    ///
    /// A request may be smaller than the machine the Generation booted: the vCPUs it did not
    /// ask for are withheld by a CPU quota applied before the launch answers. It can never be
    /// larger, because the guest has only the processors it booted with. Memory is restored at
    /// the Generation's own boot size and grown to the request, which needs the memory device
    /// and a machine that is not jailed.
    pub(super) fn misfit(
        &self,
        prepared: &PreparedGeneration,
        shape: &MachineShape,
    ) -> Option<String> {
        misfit(prepared, shape).or_else(|| {
            (self.jail.is_some() && shape.memory_mib() != boot_mib_of(prepared)).then(|| {
                format!(
                    "memory_mib {} is not the Generation's boot memory {} and a jailed machine \
                     cannot grow",
                    shape.memory_mib(),
                    boot_mib_of(prepared)
                )
            })
        })
    }
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod tests;
