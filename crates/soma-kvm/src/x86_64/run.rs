//! The bounded `KVM_RUN` loop shared by the halt guest and the kernel boot.

use std::sync::atomic::{AtomicBool, Ordering};

use kvm_ioctls::{VcpuExit, VcpuFd};

use super::{
    error::{MachineError, MachineErrorKind, Phase},
    mmio::MmioDispatch,
    ports::{PortBus, PortEvent},
};

/// How the guest stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestExit {
    /// The guest executed `hlt` and KVM returned `KVM_EXIT_HLT`.
    Halt,
    /// The guest triple-faulted or otherwise reached `KVM_EXIT_SHUTDOWN`.
    Shutdown,
    /// The guest pulsed the keyboard-controller CPU-reset line, as `reboot=k` does.
    Reset,
    /// SOMA stopped the vCPU because the expected serial sentinel arrived.
    Sentinel,
    /// SOMA kicked the vCPU out of `KVM_RUN` so its state could be captured.
    ///
    /// The guest is still live and the vCPU descriptor is handed back to the caller instead
    /// of being dropped, because a paused machine is about to be read, not released.
    Paused,
}

/// Runs the vCPU until it stops, an unexpected exit occurs, or the watchdog interrupts it.
///
/// Port I/O is dispatched through `bus`, MMIO through `mmio` when the machine has a device
/// bus; when `sentinel` is given the loop stops as soon as the captured serial output ends
/// with it. An interruption is a pause when `pause` is set and a deadline failure otherwise,
/// so the two uses of the same signal never blur into one another.
pub(crate) fn run(
    vcpu: &mut VcpuFd,
    bus: &mut PortBus,
    mut mmio: Option<&mut MmioDispatch>,
    sentinel: Option<&[u8]>,
    pause: &AtomicBool,
) -> Result<GuestExit, MachineError> {
    loop {
        match vcpu.run() {
            Ok(VcpuExit::MmioRead(address, data)) => match mmio.as_deref_mut() {
                Some(dispatch) => dispatch.read(address, data)?,
                None => return Err(unexpected(&VcpuExit::MmioRead(address, data))),
            },
            Ok(VcpuExit::MmioWrite(address, data)) => match mmio.as_deref_mut() {
                Some(dispatch) => dispatch.write(address, data)?,
                None => return Err(unexpected(&VcpuExit::MmioWrite(address, data))),
            },
            Ok(VcpuExit::IoIn(port, data)) => bus.io_in(port, data),
            Ok(VcpuExit::IoOut(port, data)) => match bus.io_out(port, data)? {
                PortEvent::Reset => return Ok(GuestExit::Reset),
                PortEvent::Continue => {
                    if sentinel.is_some_and(|expected| ends_with(bus.serial().output(), expected)) {
                        return Ok(GuestExit::Sentinel);
                    }
                }
            },
            Ok(VcpuExit::Hlt) => return Ok(GuestExit::Halt),
            Ok(VcpuExit::Shutdown) => return Ok(GuestExit::Shutdown),
            Ok(VcpuExit::Intr) => {}
            Ok(exit) => return Err(unexpected(&exit)),
            Err(error) if error.errno() == libc::EINTR => {
                if pause.load(Ordering::Acquire) {
                    return Ok(GuestExit::Paused);
                }
                return Err(MachineError::new(Phase::Run, MachineErrorKind::Timeout));
            }
            Err(error) => return Err(MachineError::os(Phase::Run, error)),
        }
    }
}

fn unexpected(exit: &VcpuExit<'_>) -> MachineError {
    MachineError::new(Phase::Run, MachineErrorKind::UnexpectedExit(classify(exit)))
}

/// True when `output` ends with `expected` followed by at most one line terminator.
fn ends_with(output: &[u8], expected: &[u8]) -> bool {
    let trimmed = output
        .strip_suffix(b"\r\n")
        .or_else(|| output.strip_suffix(b"\n"))
        .unwrap_or(output);
    !expected.is_empty() && trimmed.ends_with(expected)
}

/// Names an exit without copying guest data into error text.
fn classify(exit: &VcpuExit<'_>) -> String {
    match exit {
        VcpuExit::IoIn(port, _) => format!("io in port {port:#x}"),
        VcpuExit::IoOut(port, _) => format!("io out port {port:#x}"),
        VcpuExit::MmioRead(address, _) => format!("mmio read {address:#x}"),
        VcpuExit::MmioWrite(address, _) => format!("mmio write {address:#x}"),
        VcpuExit::Shutdown => "shutdown".to_owned(),
        VcpuExit::FailEntry(reason, cpu) => format!("fail entry reason {reason:#x} cpu {cpu}"),
        VcpuExit::InternalError => "internal error".to_owned(),
        other => format!("{other:?}")
            .split('(')
            .next()
            .unwrap_or("unknown")
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_match_tolerates_one_trailing_newline() {
        assert!(ends_with(b"boot log\nSOMA-BOOT-ab\n", b"SOMA-BOOT-ab"));
        assert!(ends_with(b"SOMA-BOOT-ab\r\n", b"SOMA-BOOT-ab"));
        assert!(ends_with(b"SOMA-BOOT-ab", b"SOMA-BOOT-ab"));
        assert!(!ends_with(b"SOMA-BOOT-ab\n\n", b"SOMA-BOOT-ab"));
        assert!(!ends_with(b"SOMA-BOOT-a", b"SOMA-BOOT-ab"));
        assert!(!ends_with(b"anything", b""));
    }

    #[test]
    fn classification_omits_guest_bytes() {
        let mut data = [0xde_u8, 0xad];
        assert_eq!(classify(&VcpuExit::IoOut(0x80, &data)), "io out port 0x80");
        assert_eq!(
            classify(&VcpuExit::IoIn(0x60, &mut data)),
            "io in port 0x60"
        );
        assert_eq!(classify(&VcpuExit::Shutdown), "shutdown");
        assert_eq!(classify(&VcpuExit::Hlt), "Hlt");
    }
}
