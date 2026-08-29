mod fdt;
mod gic;
mod host;
mod layout;
mod uart;
mod vcpu;
mod watchdog;

#[cfg(test)]
mod tests;

use std::{error::Error, fmt, fs::File, io::Read, path::Path, time::Duration};

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use linux_loader::loader::{KernelLoader, pe::PE};
use vm_memory::{Address, Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryMmap};

use self::{
    layout::{BootLayout, FDT_MAX_SIZE, KERNEL_BASE, RAM_BASE, RAM_SIZE},
    uart::Uart,
};

const ARM64_BOOT_SENTINEL: &str = "SOMA_ARM64_OK";
const BOOT_TIMEOUT: Duration = Duration::from_secs(30);

struct Arm64BootEvidence {
    console: Vec<u8>,
}

impl Arm64BootEvidence {
    #[must_use]
    fn console(&self) -> &[u8] {
        &self.console
    }
}

impl fmt::Debug for Arm64BootEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Arm64BootEvidence")
            .field("console_len", &self.console.len())
            .finish()
    }
}

#[derive(Debug)]
struct Arm64BootError {
    message: String,
}

impl Arm64BootError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn at(stage: &str, error: impl fmt::Display) -> Self {
        Self::message(format!("{stage}: {error}"))
    }
}

impl fmt::Display for Arm64BootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for Arm64BootError {}

/// Cold-boots explicit Linux ARM64 fixtures until the expected serial sentinel is observed.
///
/// This is an experimental KVM boot proof with one vCPU and 128 MiB of RAM.
/// Sentinel provenance depends on the caller's trust in the explicit fixture files.
/// A returned sentinel is not an authenticated-ready or performance result.
///
/// # Process containment
///
/// This crate-internal proof must run as one exact ignored test in a dedicated test process.
/// During execution it exclusively reserves `SIGRTMIN + 7`, temporarily replaces that signal's
/// process-wide handler, and restores the previous handler after the vCPU thread has joined.
/// The vCPU worker blocks that signal normally and asks KVM to unblock it only inside `KVM_RUN`.
/// If watchdog setup, the targeted kick, or the bounded join cannot contain the worker, the process
/// aborts rather than releasing memory that a live vCPU could still access.
///
/// # Errors
///
/// Returns an error when fixture validation or any required ARM64 KVM boot stage fails.
fn boot_arm64_fixture(
    kernel_path: &Path,
    initramfs_path: &Path,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    boot_with(
        kernel_path,
        initramfs_path,
        ARM64_BOOT_SENTINEL.as_bytes(),
        BOOT_TIMEOUT,
    )
}

fn boot_with(
    kernel_path: &Path,
    initramfs_path: &Path,
    expected_sentinel: &'static [u8],
    timeout: Duration,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    if expected_sentinel.is_empty() {
        return Err(Arm64BootError::message("expected sentinel is empty"));
    }
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
    let device_tree =
        fdt::build(layout).map_err(|error| Arm64BootError::at("build ARM64 device tree", error))?;
    let fdt_limit = usize::try_from(FDT_MAX_SIZE)
        .map_err(|error| Arm64BootError::at("convert FDT size limit", error))?;
    if device_tree.len() > fdt_limit {
        return Err(Arm64BootError::message("device tree exceeds two MiB"));
    }
    memory
        .write_slice(&device_tree, GuestAddress(layout.fdt_start))
        .map_err(|error| Arm64BootError::at("copy device tree into guest RAM", error))?;

    let kvm = Kvm::new().map_err(|error| Arm64BootError::at("open /dev/kvm", error))?;
    host::validate(&kvm).map_err(|error| Arm64BootError::at("validate ARM64 boot host", error))?;
    let vm = kvm
        .create_vm()
        .map_err(|error| Arm64BootError::at("create ARM64 VM", error))?;
    register_memory(&vm, &memory)?;
    let vcpu = vm
        .create_vcpu(0)
        .map_err(|error| Arm64BootError::at("create vCPU 0", error))?;
    vcpu::initialize(&vm, &vcpu, kernel.kernel_load.raw_value(), layout.fdt_start)
        .map_err(|error| Arm64BootError::at("initialize vCPU 0", error))?;
    let _gic = gic::create(&vm)
        .map_err(|error| Arm64BootError::at("create and initialize GICv3", error))?;
    watchdog::run(vcpu, expected_sentinel, timeout)
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
    // SAFETY: Slot 0 uniquely covers the checked 128 MiB mapping. `memory` was created before the
    // KVM handles, is never resized, and therefore outlives the VM and vCPU that can access it.
    unsafe { vm.set_user_memory_region(region) }
        .map_err(|error| Arm64BootError::at("register guest RAM with KVM", error))
}

fn run_vcpu(
    mut vcpu: VcpuFd,
    expected_sentinel: &'static [u8],
) -> Result<Arm64BootEvidence, Arm64BootError> {
    let mut uart = Uart::new(expected_sentinel);
    loop {
        match vcpu
            .run()
            .map_err(|error| Arm64BootError::at("run vCPU 0", error))?
        {
            VcpuExit::MmioRead(address, data) => uart
                .read(address, data)
                .map_err(|error| Arm64BootError::at("emulate serial MMIO read", error))?,
            VcpuExit::MmioWrite(address, data) => {
                if uart
                    .write(address, data)
                    .map_err(|error| Arm64BootError::at("emulate serial MMIO write", error))?
                {
                    return Ok(Arm64BootEvidence {
                        console: uart.into_console(),
                    });
                }
            }
            VcpuExit::Intr => {}
            exit => {
                return Err(Arm64BootError::message(format!(
                    "vCPU exited before the fixture sentinel: {exit:?}"
                )));
            }
        }
    }
}
