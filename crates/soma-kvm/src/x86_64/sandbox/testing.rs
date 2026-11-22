//! A small CPU-only KVM guest for testing resident machine transitions.

use std::{
    fs::File,
    io::{Seek as _, Write as _},
};

use super::SandboxMachine;
use crate::x86_64::{
    DeviceIdentity, SandboxConfig, SandboxDisks, elf::synthetic::SyntheticElf, guest::HALT_PROGRAM,
    layout::KERNEL_START,
};
use crate::{DeviceSet, MachineContract};

pub(super) fn spinning_machine(volume: Option<File>) -> (SandboxMachine, u32) {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let counter_address = entry + 256;

    let mut elf = SyntheticElf::kernel(entry);
    let mut program = HALT_PROGRAM[..HALT_PROGRAM.len() - 1].to_vec();
    // Replace hlt with a spin loop that increments a word in guest memory.
    program.extend([0xff, 0x05]); // inc dword ptr [address]
    program.extend(counter_address.to_le_bytes());
    program.extend([0xeb, 0xf8]); // jmp back to inc
    elf.segments[0].data = program;
    let mut kernel = tempfile::tempfile().unwrap();
    kernel.write_all(&elf.build()).unwrap();
    kernel.rewind().unwrap();
    let initramfs = tempfile::tempfile().unwrap();
    initramfs.set_len(4096).unwrap();
    let root = tempfile::NamedTempFile::new().unwrap();
    root.as_file().set_len(4096).unwrap();

    let contract = MachineContract::V1;
    let devices = DeviceSet::new(false, false).with_volume(volume.is_some());
    let machine = SandboxMachine::create(SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks {
            root: File::open(root.path()).unwrap(),
            overlay: None,
            volume,
            volume_placeholder: false,
        },
        identity: DeviceIdentity {
            guest_cid: 3,
            guest_mac: [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01],
        },
        ram_bytes: contract.min_memory_bytes(),
        vcpus: 1,
        contract,
        devices,
        hotplug: None,
    })
    .expect("create KVM guest");
    (machine, counter_address)
}
