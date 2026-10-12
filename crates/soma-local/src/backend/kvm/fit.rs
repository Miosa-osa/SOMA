//! Whether a Generation can give a request, whatever process serves it.

use soma::MachineShape;

use super::{hotplug, prepared::PreparedGeneration};

pub(super) fn boot_mib_of(prepared: &PreparedGeneration) -> u64 {
    prepared.manifest.shape.memory_bytes / (1024 * 1024)
}

/// Whether the Generation can give the request, whatever process serves it.
///
/// A host that jails its machines is stricter (memory must be exactly the boot size), and that
/// part is decided where the jail is known. The parent of a machine host asks this before it
/// claims a prepared machine, so a request that can never be served does not consume one or
/// count as demand.
pub(super) fn request_fits(prepared: &PreparedGeneration, shape: &MachineShape) -> bool {
    let booted = &prepared.manifest.shape;
    let boot_mib = boot_mib_of(prepared);
    let vcpus = shape.vcpu_count() >= 1 && shape.vcpu_count() <= booted.vcpu_count;
    let memory = shape.memory_mib() == boot_mib
        || (shape.memory_mib() > boot_mib && hotplug::room_for(prepared, shape.memory_mib()));
    let storage = prepared
        .manifest
        .template
        .writable_storage_bytes
        .is_multiple_of(1024 * 1024)
        && shape.storage_mib() == prepared.manifest.template.writable_storage_bytes / (1024 * 1024);
    vcpus && memory && storage
}
