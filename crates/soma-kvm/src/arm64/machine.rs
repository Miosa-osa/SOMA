use std::{fs::File, io::Read, path::Path};

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::{DeviceFd, Kvm, VcpuFd, VmFd};
use linux_loader::loader::{KernelLoader, pe::PE};
use vm_memory::{Address, Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryMmap};

use super::{
    Arm64BootError, fdt, gic, host,
    layout::{BootLayout, FDT_MAX_SIZE, KERNEL_BASE, RAM_BASE, RAM_SIZE},
    vcpu,
};

#[derive(Clone, Copy)]
pub(crate) enum DeviceProfile {
    ConsoleOnly,
    Command,
}

pub(crate) struct Machine {
    pub(crate) vcpu: VcpuFd,
    pub(crate) gic: DeviceFd,
    pub(crate) vm: VmFd,
    pub(crate) memory: GuestMemoryMmap<()>,
}

pub(crate) fn prepare(
    kernel_path: &Path,
    initramfs_path: &Path,
    profile: DeviceProfile,
) -> Result<Machine, Arm64BootError> {
    let initramfs = read_initramfs(initramfs_path)?;
    let layout = BootLayout::new(initramfs.len()).map_err(Arm64BootError::message)?;
    let ram_size = usize::try_from(RAM_SIZE)
        .map_err(|error| Arm64BootError::at("convert guest RAM size", error))?;
    let memory = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(RAM_BASE), ram_size)])
        .map_err(|error| Arm64BootError::at("map 128 MiB guest RAM", error))?;

    let kernel = load_kernel(kernel_path, &memory)?;
    if kernel.kernel_end > layout.initrd_start {
        return Err(Arm64BootError::message(
            "kernel overlaps the reserved initramfs region",
        ));
    }
    memory
        .write_slice(&initramfs, GuestAddress(layout.initrd_start))
        .map_err(|error| Arm64BootError::at("copy initramfs into guest RAM", error))?;
    let include_control = matches!(profile, DeviceProfile::Command);
    let device_tree = fdt::build(layout, include_control)
        .map_err(|error| Arm64BootError::at("build ARM64 device tree", error))?;
    let fdt_limit = usize::try_from(FDT_MAX_SIZE)
        .map_err(|error| Arm64BootError::at("convert FDT size limit", error))?;
    if device_tree.len() > fdt_limit {
        return Err(Arm64BootError::message("device tree exceeds two MiB"));
    }
    memory
        .write_slice(&device_tree, GuestAddress(layout.fdt_start))
        .map_err(|error| Arm64BootError::at("copy device tree into guest RAM", error))?;

    let kvm = Kvm::new().map_err(|error| Arm64BootError::at("open /dev/kvm", error))?;
    match profile {
        DeviceProfile::ConsoleOnly => host::validate(&kvm),
        DeviceProfile::Command => host::validate_command(&kvm),
    }
    .map_err(|error| Arm64BootError::at("validate ARM64 boot host", error))?;
    let vm = kvm
        .create_vm()
        .map_err(|error| Arm64BootError::at("create ARM64 VM", error))?;
    register_memory(&vm, &memory)?;
    let vcpu = vm
        .create_vcpu(0)
        .map_err(|error| Arm64BootError::at("create vCPU 0", error))?;
    vcpu::initialize(&vm, &vcpu, kernel.kernel_load.raw_value(), layout.fdt_start)
        .map_err(|error| Arm64BootError::at("initialize vCPU 0", error))?;
    let gic = gic::create(&vm)
        .map_err(|error| Arm64BootError::at("create and initialize GICv3", error))?;
    Ok(Machine {
        vcpu,
        gic,
        vm,
        memory,
    })
}

fn read_initramfs(path: &Path) -> Result<Vec<u8>, Arm64BootError> {
    let mut file = File::open(path)
        .map_err(|error| Arm64BootError::at("open explicit initramfs fixture", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| Arm64BootError::at("inspect initramfs fixture", error))?;
    if !metadata.is_file() {
        return Err(Arm64BootError::message(
            "initramfs fixture is not a regular file",
        ));
    }
    let size = usize::try_from(metadata.len())
        .map_err(|error| Arm64BootError::at("convert initramfs size", error))?;
    BootLayout::new(size).map_err(Arm64BootError::message)?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)
        .map_err(|error| Arm64BootError::at("read initramfs fixture", error))?;
    Ok(bytes)
}

fn load_kernel(
    path: &Path,
    memory: &GuestMemoryMmap<()>,
) -> Result<linux_loader::loader::KernelLoaderResult, Arm64BootError> {
    let mut file = File::open(path)
        .map_err(|error| Arm64BootError::at("open explicit kernel fixture", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| Arm64BootError::at("inspect kernel fixture", error))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() >= RAM_SIZE {
        return Err(Arm64BootError::message(
            "kernel fixture must be a nonempty regular file smaller than guest RAM",
        ));
    }
    PE::load(memory, Some(GuestAddress(KERNEL_BASE)), &mut file, None)
        .map_err(|error| Arm64BootError::at("load Linux ARM64 Image", error))
}

#[allow(unsafe_code)]
fn register_memory(vm: &VmFd, memory: &GuestMemoryMmap<()>) -> Result<(), Arm64BootError> {
    let host_pointer = memory
        .get_host_address(GuestAddress(RAM_BASE))
        .map_err(|error| Arm64BootError::at("resolve guest RAM host address", error))?;
    let userspace_addr = u64::try_from(host_pointer.addr())
        .map_err(|error| Arm64BootError::at("convert guest RAM host address", error))?;
    let region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: RAM_BASE,
        memory_size: RAM_SIZE,
        userspace_addr,
        flags: 0,
    };
    // SAFETY: Slot 0 uniquely covers this immutable 128 MiB host mapping. `Machine` cleanup and
    // the watchdog join ensure the VM and vCPU are gone before `memory` can be released.
    unsafe { vm.set_user_memory_region(region) }
        .map_err(|error| Arm64BootError::at("register guest RAM with KVM", error))
}
