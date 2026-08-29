mod command;
mod control_uart;
mod executor;
mod fdt;
mod gic;
mod host;
mod layout;
mod machine;
mod protocol;
mod response;
mod uart;
mod vcpu;
mod watchdog;

#[cfg(test)]
mod tests;

use std::{error::Error, fmt, path::Path, time::Duration};

use kvm_ioctls::{VcpuExit, VcpuFd};

use self::{machine::DeviceProfile, uart::Uart};

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
    let machine = machine::prepare(kernel_path, initramfs_path, DeviceProfile::ConsoleOnly)?;
    let machine::Machine {
        vcpu,
        vm,
        gic,
        memory,
    } = machine;
    let result = watchdog::run(vcpu, expected_sentinel, timeout);
    drop(gic);
    drop(vm);
    drop(memory);
    result
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
