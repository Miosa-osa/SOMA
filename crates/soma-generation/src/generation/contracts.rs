use soma_kvm::DeviceSet;

use super::artifacts::Sha256Digest;

/// The memory-slot layout version the `x86_64` machine contract fixes.
///
/// Version 1 is guest RAM in slot 0 and the dedicated non-snapshot launch page in slot 1.
pub const MEMORY_SLOT_LAYOUT_VERSION: u16 = 1;

/// The launch-page layout version the guest protocol fixes.
///
/// This restates `soma_guest::LAUNCH_PAGE_SCHEMA_VERSION`; a test binds the two values, and a
/// Generation built for another schema is rejected by compatibility verification.
pub const LAUNCH_PAGE_LAYOUT_VERSION: u16 = 3;

/// The repair-policy version the readiness contract fixes.
pub const REPAIR_POLICY_VERSION: u16 = 1;

/// The snapshot format version certification will bind.
pub const SNAPSHOT_FORMAT_VERSION: u16 = 2;

/// The snapshot capture-point version certification will bind.
pub const SNAPSHOT_CAPTURE_POINT_VERSION: u16 = 1;

/// One versioned contract identity bound into the Generation manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractBinding {
    /// The contract version number.
    pub version: u16,
    /// The SHA-256 digest of the canonical contract statement.
    pub digest: Sha256Digest,
}

impl ContractBinding {
    fn of(version: u16, statement: &[u8]) -> Self {
        Self {
            version,
            digest: Sha256Digest::of(statement),
        }
    }
}

/// The canonical machine-readable statement of `x86_64` machine contract v1.
///
/// The digest of these bytes, rather than of the prose document, is what the manifest binds.
pub const MACHINE_CONTRACT_V1: &[u8] = b"soma-x86_64-machine-contract-v1\n\
boot=pvh-direct elf=ET_EXEC note=XEN_ELFNOTE_PHYS32_ENTRY\n\
vcpu=1 ram-min=134217728 ram-max=3221225472 ram-step=4096\n\
start-info=0x6000 memmap=0x7000 modules=0x8000 cmdline=0x9000 cmdline-max=8191\n\
low-reserved=0x0-0x5fff workspace=0xb000-0x9ffff legacy-hole=0xa0000-0xfffff\n\
loader-gap=0x100000-0xffffff kernel-min-paddr=0x1000000 physical-start=0x1000000\n\
initramfs=top-down-page-aligned modules-max=1\n\
base-cmdline=console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests\n";

/// The header every device-contract statement opens with, before its present slots.
const DEVICE_CONTRACT_HEADER: &str = "soma-minimal-device-surface-v1\n\
transport=virtio-mmio version=2 magic=0x74726976 features=VIRTIO_F_VERSION_1 queues=split\n\
irqchip=in-kernel ioapic edge-triggered no-shared-gsi\n";
/// The trailer, listing what version 1 has no device for at all.
const DEVICE_CONTRACT_TRAILER: &str =
    "excluded=pci,pcie,msi,msix,iommu,packed-ring,vhost,console,balloon,memory,fs,scsi,hotplug\n";
/// One line per slot, in table order; a Generation states only the slots it has.
const DEVICE_CONTRACT_SLOTS: [&str; 5] = [
    "slot0 mmio=0xd0000000-0xd0000fff gsi=5 device=block id=2 role=immutable-root \
queues=request:256\n",
    "slot1 mmio=0xd0001000-0xd0001fff gsi=6 device=block id=2 role=writable-overlay \
queues=request:256\n",
    "slot2 mmio=0xd0002000-0xd0002fff gsi=7 device=net id=1 queues=receive:256,transmit:256\n",
    "slot3 mmio=0xd0003000-0xd0003fff gsi=8 device=vsock id=19 \
queues=receive:256,transmit:256,event:64\n",
    "slot4 mmio=0xd0004000-0xd0004fff gsi=9 device=entropy id=4 queues=request:64\n",
];

/// The canonical machine-readable statement of the device contract for one device set.
///
/// The five slots are the maximum, not the minimum: a Generation that declared no writable
/// storage states no overlay slot and one that declared no egress states no network slot. The
/// statement therefore says which machine this Generation is, and its digest is what refuses a
/// snapshot of a different one.
#[must_use]
pub fn device_contract_statement(devices: DeviceSet) -> Vec<u8> {
    let mut statement = String::from(DEVICE_CONTRACT_HEADER);
    for slot in devices.present() {
        statement.push_str(DEVICE_CONTRACT_SLOTS[usize::from(slot.index())]);
    }
    statement.push_str(DEVICE_CONTRACT_TRAILER);
    statement.into_bytes()
}

/// The canonical statement of CPU template v1.
///
/// Ticket-level CPUID and MSR masks are not yet defined, so version 1 binds only the
/// selection rule; changing the rule or defining masks requires a new version.
pub const CPU_TEMPLATE_V1: &[u8] = b"soma-cpu-template-v1\n\
source=KVM_GET_SUPPORTED_CPUID apply=KVM_SET_CPUID2 vcpu=1\n\
masks=undefined-pending-ticket status=declaration-only\n";

/// The fixed readiness command executed after authenticated repair.
pub const READINESS_COMMAND: &[u8] = b"/proc/self/exe --soma-ready-probe-v1";

/// Returns the machine contract v1 binding.
#[must_use]
pub fn machine_contract_v1() -> ContractBinding {
    ContractBinding::of(1, MACHINE_CONTRACT_V1)
}

/// Returns the device contract v1 binding for one device set.
#[must_use]
pub fn device_contract_v1(devices: DeviceSet) -> ContractBinding {
    ContractBinding::of(1, &device_contract_statement(devices))
}

/// Returns the CPU template v1 binding.
#[must_use]
pub fn cpu_template_v1() -> ContractBinding {
    ContractBinding::of(1, CPU_TEMPLATE_V1)
}

/// Returns the complete generated kernel command line for Generation profile v1.
///
/// Every field is fixed and ordered; no caller text participates. It is composed by the machine
/// layer rather than restated here, because the manifest binds the line the machine will
/// actually boot with and a second spelling of it could only ever drift from the first.
#[must_use]
pub fn kernel_command_line_v1(devices: DeviceSet) -> Vec<u8> {
    soma_kvm::generation_command_line(devices).into_bytes()
}

/// Returns the digest of the fixed readiness command bytes.
#[must_use]
pub fn readiness_command_digest() -> Sha256Digest {
    Sha256Digest::of(READINESS_COMMAND)
}
