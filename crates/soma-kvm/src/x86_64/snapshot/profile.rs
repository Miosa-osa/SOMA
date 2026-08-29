//! What this host and this implementation promise, expressed the way the compatibility
//! contract compares it.
//!
//! Every value here is derived from the fixed machine and device contracts or read from the
//! live KVM, never from the snapshot being checked: a profile built from the snapshot would
//! make the comparison vacuous.

use kvm_bindings::{CpuId, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Cap, Kvm};

use super::error::SnapshotError;
use crate::snapshot::{
    Digest, Hasher,
    compatibility::{DeviceExpectation, HostProfile},
    device_state::{DeviceKind, MAX_QUEUES},
    kvm_state::CpuidEntries,
    manifest::{Architecture, HostCapability, HostRequirements, PageSize, SCHEMA_VERSION},
};
use crate::virtio::{
    BLOCK_QUEUE_MAX, BlockRole, NET_FEATURES, NET_QUEUE_MAX, RNG_FEATURES, RNG_QUEUE_MAX, Slot,
    VSOCK_FEATURES, VSOCK_QUEUE_MAX,
};
use crate::x86_64::{cmdline, cpuid, launch_page, layout};

/// Version of the bounded guest control protocol the certified agent speaks.
pub(in crate::x86_64) const GUEST_PROTOCOL_VERSION: u16 = 1;
/// Guest RAM slot plus the separate launch-page slot.
pub(in crate::x86_64) const REQUIRED_MEMORY_SLOTS: u16 = 2;
/// The version 1 machine runs exactly one vCPU.
pub(in crate::x86_64) const VCPU_COUNT: u16 = 1;
/// Largest `kvm_xsave` region the certified state format carries.
pub(in crate::x86_64) const XSAVE_LIMIT: i32 = 4096;

/// KVM capabilities a restoring host must report, in ascending wire order.
const REQUIRED: [(HostCapability, Cap); 12] = [
    (HostCapability::UserMemory, Cap::UserMemory),
    (HostCapability::IrqChip, Cap::Irqchip),
    (HostCapability::IrqFd, Cap::Irqfd),
    (HostCapability::IoEventFd, Cap::Ioeventfd),
    (HostCapability::ImmediateExit, Cap::ImmediateExit),
    (HostCapability::Xsave, Cap::Xsave),
    (HostCapability::Xcrs, Cap::Xcrs),
    (HostCapability::VcpuEvents, Cap::VcpuEvents),
    (HostCapability::MpState, Cap::MpState),
    (HostCapability::AdjustClock, Cap::AdjustClock),
    (HostCapability::SetTssAddr, Cap::SetTssAddr),
    (HostCapability::Pit2, Cap::Pit2),
];

/// Digest of the fixed guest-physical layout, boot ABI, and command line.
///
/// It changes whenever the machine the snapshot was taken on stops being the machine the
/// snapshot would be restored onto, which is exactly when restore must fail closed.
#[must_use]
pub(in crate::x86_64) fn machine_contract() -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(b"SOMA-x86_64-machine-contract-v1\0");
    for value in [
        layout::TSS_ADDRESS,
        layout::START_INFO_ADDRESS,
        layout::MEMMAP_ADDRESS,
        layout::MODULE_ADDRESS,
        layout::CMDLINE_ADDRESS,
        layout::KERNEL_START,
        launch_page::LAUNCH_PAGE_GPA,
        u64::from(launch_page::LAUNCH_PAGE_SLOT),
    ] {
        hasher.update(&value.to_be_bytes());
    }
    hasher.update(cmdline::compose_generation().as_bytes());
    hasher.finish()
}

/// Digest of the fixed five-slot device surface: addresses, interrupts, ids, queues, features.
#[must_use]
pub(in crate::x86_64) fn device_contract() -> Digest {
    let mut hasher = Hasher::new();
    hasher.update(b"SOMA-device-surface-v1\0");
    for slot in Slot::ALL {
        hasher.update(&slot.base().to_be_bytes());
        hasher.update(&slot.gsi().to_be_bytes());
        hasher.update(&slot.device_id().to_be_bytes());
        hasher.update(&slot.queue_count().to_be_bytes());
        let expectation = expectation(slot);
        hasher.update(&expectation.negotiated_features.to_be_bytes());
        for limit in expectation.queue_limits {
            hasher.update(&limit.to_be_bytes());
        }
    }
    hasher.finish()
}

/// The feature allowlist and queue limits this implementation offers on one slot.
#[must_use]
pub(in crate::x86_64) fn expectation(slot: Slot) -> DeviceExpectation {
    let mut queue_limits = [0_u16; MAX_QUEUES];
    let (kind, negotiated_features, limits): (DeviceKind, u64, &[u16]) = match slot {
        Slot::Root => (
            DeviceKind::RootBlock,
            BlockRole::ImmutableRoot.features(),
            &BLOCK_QUEUE_MAX,
        ),
        Slot::Overlay => (
            DeviceKind::OverlayBlock,
            BlockRole::PrivateOverlay.features(),
            &BLOCK_QUEUE_MAX,
        ),
        Slot::Net => (DeviceKind::Net, NET_FEATURES, &NET_QUEUE_MAX),
        Slot::Vsock => (DeviceKind::Vsock, VSOCK_FEATURES, &VSOCK_QUEUE_MAX),
        Slot::Rng => (DeviceKind::Rng, RNG_FEATURES, &RNG_QUEUE_MAX),
    };
    for (limit, value) in queue_limits.iter_mut().zip(limits) {
        *limit = *value;
    }
    DeviceExpectation {
        kind,
        negotiated_features,
        queue_limits,
    }
}

/// The filtered CPUID template this host would install, and its digest.
///
/// # Errors
///
/// Returns the KVM failure, or the template rejection when the host cannot provide a leaf
/// the contract requires.
pub(in crate::x86_64) fn cpu_template(kvm: &Kvm) -> Result<(CpuId, Digest), SnapshotError> {
    let mut template = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(|error| SnapshotError::ioctl("KVM_GET_SUPPORTED_CPUID", error))?;
    cpuid::apply_template(&mut template)?;
    let entries = CpuidEntries::try_from(&template)?;
    let mut hasher = Hasher::new();
    hasher.update(b"SOMA-cpu-template-v1\0");
    for entry in entries.entries() {
        for word in [
            entry.function,
            entry.index,
            entry.flags,
            entry.eax,
            entry.ebx,
            entry.ecx,
            entry.edx,
        ] {
            hasher.update(&word.to_be_bytes());
        }
    }
    Ok((template, hasher.finish()))
}

/// The requirement block a captured manifest carries for its restoring hosts.
///
/// # Errors
///
/// Returns the requirement rejection, which the fixed ascending list cannot trigger.
pub(in crate::x86_64) fn requirements() -> Result<HostRequirements, SnapshotError> {
    HostRequirements::new(
        u32::try_from(crate::KVM_API_VERSION).unwrap_or(0),
        REQUIRED.iter().map(|(capability, _)| *capability).collect(),
        REQUIRED_MEMORY_SLOTS,
    )
    .map_err(|error| SnapshotError::Manifest(error.into()))
}

/// Everything the live host promises, read from KVM and the fixed contracts.
///
/// # Errors
///
/// Returns the KVM failure, or [`SnapshotError::XsaveTooLarge`] when the host holds more
/// extended state than the certified format carries.
pub(in crate::x86_64) fn host_profile(
    kvm: &Kvm,
    memory_bytes: u64,
) -> Result<HostProfile, SnapshotError> {
    let xsave = kvm.check_extension_int(Cap::Xsave2);
    if xsave > XSAVE_LIMIT {
        return Err(SnapshotError::XsaveTooLarge(xsave));
    }
    let capabilities = REQUIRED
        .iter()
        .filter(|(_, cap)| kvm.check_extension(*cap))
        .map(|(capability, _)| *capability)
        .collect();
    let (_, cpu_template) = cpu_template(kvm)?;
    let devices = Slot::ALL.map(expectation);
    Ok(HostProfile {
        schema_version: SCHEMA_VERSION,
        architecture: Architecture::X86_64,
        page_size: PageSize::FOUR_KIB,
        kvm_api_version: u32::try_from(kvm.get_api_version()).unwrap_or(0),
        capabilities,
        memory_slots: u16::try_from(kvm.get_nr_memslots()).unwrap_or(u16::MAX),
        machine_contract: machine_contract(),
        device_contract: device_contract(),
        cpu_template,
        vcpu_count: VCPU_COUNT,
        memory_bytes,
        guest_protocol_version: GUEST_PROTOCOL_VERSION,
        devices,
    })
}
