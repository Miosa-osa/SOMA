//! Host profile versus manifest requirement check.
//!
//! Every comparison is exact equality or explicit containment, the first mismatch is
//! reported as a typed reason, and nothing is defaulted, downgraded, or renegotiated.
//! Constant-size header checks run before any section payload is decoded, so a large
//! artifact is never mapped for an incompatible snapshot.

mod reason;
#[cfg(test)]
mod tests;

pub use reason::Incompatibility;

use super::{
    Digest,
    device_state::{DEVICE_COUNT, DeviceKind, DeviceState, MAX_QUEUES},
    kvm_state::VmState,
    manifest::{Architecture, HostCapability, Manifest, PageSize, SCHEMA_VERSION},
    section::SectionRole,
};

/// What the host implementation expects of one device slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceExpectation {
    pub kind: DeviceKind,
    pub negotiated_features: u64,
    pub queue_limits: [u16; MAX_QUEUES],
}

/// Everything a restoring host knows about itself and its device implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostProfile {
    pub schema_version: u16,
    pub architecture: Architecture,
    pub page_size: PageSize,
    pub kvm_api_version: u32,
    pub capabilities: Vec<HostCapability>,
    pub memory_slots: u16,
    pub machine_contract: Digest,
    pub device_contract: Digest,
    pub cpu_template: Digest,
    pub vcpu_count: u16,
    pub memory_bytes: u64,
    pub guest_protocol_version: u16,
    /// What this host expects of each slot, or `None` where its Generation declared no device.
    pub devices: [Option<DeviceExpectation>; DEVICE_COUNT as usize],
}

/// Checks the constant-size header, then the VM layout, then every device slot.
///
/// A slot this host has no device for must carry no section either. Without that, a snapshot
/// captured from a machine that had a writable overlay would restore onto a machine built
/// without one, and the guest inside it would come back believing it still has a disk.
///
/// # Errors
///
/// Returns the first [`Incompatibility`] in the fixed check order.
pub fn check(host: &HostProfile, manifest: &Manifest) -> Result<(), Incompatibility> {
    check_header(host, manifest)?;
    check_vm_layout(manifest)?;
    for slot in 0..DEVICE_COUNT {
        if host
            .devices
            .get(usize::from(slot))
            .is_some_and(Option::is_some)
        {
            check_device(host, manifest, slot)?;
        } else if let Some(role) =
            device_role(slot).filter(|role| manifest.section(*role).is_some())
        {
            return Err(Incompatibility::UnexpectedSection(role));
        }
    }
    Ok(())
}

/// The manifest section one device slot occupies.
fn device_role(slot: u8) -> Option<SectionRole> {
    SectionRole::ALL
        .into_iter()
        .find(|role| role.device_slot() == Some(slot))
}

/// Checks only the constant-size header fields.
///
/// # Errors
///
/// Returns the first header [`Incompatibility`].
pub fn check_header(host: &HostProfile, manifest: &Manifest) -> Result<(), Incompatibility> {
    let header = manifest.header();
    exact(host.schema_version, SCHEMA_VERSION, |expected, actual| {
        Incompatibility::SchemaVersion { expected, actual }
    })?;
    exact(
        host.architecture,
        header.architecture,
        |expected, actual| Incompatibility::Architecture { expected, actual },
    )?;
    exact(
        host.page_size.get(),
        header.page_size.get(),
        |expected, actual| Incompatibility::PageSize { expected, actual },
    )?;
    exact(
        host.memory_bytes,
        header.memory.size(),
        |expected, actual| Incompatibility::MemoryLayout { expected, actual },
    )?;
    exact(host.vcpu_count, header.vcpu_count, |expected, actual| {
        Incompatibility::VcpuCount { expected, actual }
    })?;
    exact(
        host.cpu_template,
        header.cpu_template,
        |expected, actual| Incompatibility::CpuTemplate { expected, actual },
    )?;
    exact(
        host.kvm_api_version,
        header.host.kvm_api_version(),
        |expected, actual| Incompatibility::KvmApiVersion { expected, actual },
    )?;
    for required in header.host.capabilities() {
        if !host.capabilities.contains(required) {
            return Err(Incompatibility::MissingCapability(*required));
        }
    }
    if host.memory_slots < header.host.min_memory_slots() {
        return Err(Incompatibility::MemorySlots {
            required: header.host.min_memory_slots(),
            available: host.memory_slots,
        });
    }
    exact(
        host.machine_contract,
        header.machine_contract,
        |expected, actual| Incompatibility::MachineContract { expected, actual },
    )?;
    exact(
        host.device_contract,
        header.device_contract,
        |expected, actual| Incompatibility::DeviceContract { expected, actual },
    )?;
    exact(
        host.guest_protocol_version,
        header.guest_protocol_version,
        |expected, actual| Incompatibility::GuestProtocolVersion { expected, actual },
    )
}

/// Requires the certified slot layout to cover exactly the memory object.
///
/// # Errors
///
/// Returns [`Incompatibility::MalformedVmState`] or [`Incompatibility::MemoryLayout`].
pub fn check_vm_layout(manifest: &Manifest) -> Result<(), Incompatibility> {
    let section = manifest
        .section(SectionRole::VmState)
        .ok_or(Incompatibility::MissingSection(SectionRole::VmState))?;
    let vm = VmState::decode(section.payload()).map_err(Incompatibility::MalformedVmState)?;
    let expected = manifest.header().memory.size();
    let covered = vm.total_bytes();
    let in_bounds = vm.slots().iter().all(|slot| {
        slot.memory_offset
            .checked_add(slot.size)
            .is_some_and(|end| end <= expected)
    });
    if covered != expected || !in_bounds {
        return Err(Incompatibility::MemoryLayout {
            expected,
            actual: covered,
        });
    }
    Ok(())
}

/// Decodes one device section and compares queue limits and negotiated features with the
/// host expectation for that slot.
///
/// # Errors
///
/// Returns the first device [`Incompatibility`] for `slot`.
pub fn check_device(
    host: &HostProfile,
    manifest: &Manifest,
    slot: u8,
) -> Result<(), Incompatibility> {
    let expectation = host
        .devices
        .get(usize::from(slot))
        .and_then(|expectation| expectation.as_ref())
        .filter(|expectation| expectation.kind.slot() == slot)
        .ok_or(Incompatibility::NoExpectationForSlot(slot))?;
    let role = device_role(slot).ok_or(Incompatibility::NoExpectationForSlot(slot))?;
    let section = manifest
        .section(role)
        .ok_or(Incompatibility::MissingSection(role))?;
    let state = DeviceState::decode_for_slot(slot, section.payload())
        .map_err(|error| Incompatibility::MalformedDevice { slot, error })?;
    for (index, queue) in state.queues().iter().enumerate() {
        let expected = expectation.queue_limits[index];
        if queue.max_size != expected {
            return Err(Incompatibility::QueueLimit {
                slot,
                queue: u8::try_from(index).unwrap_or(u8::MAX),
                expected,
                actual: queue.max_size,
            });
        }
    }
    exact(
        expectation.negotiated_features,
        state.negotiated_features(),
        |expected, actual| Incompatibility::FeatureNegotiation {
            slot,
            expected,
            actual,
        },
    )
}

fn exact<T: PartialEq + Copy>(
    expected: T,
    actual: T,
    reason: impl FnOnce(T, T) -> Incompatibility,
) -> Result<(), Incompatibility> {
    if expected == actual {
        Ok(())
    } else {
        Err(reason(expected, actual))
    }
}
