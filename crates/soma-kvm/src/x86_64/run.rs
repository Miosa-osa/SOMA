//! The bounded `KVM_RUN` loop for the halt guest.

use kvm_ioctls::{VcpuExit, VcpuFd};

use super::{
    error::{HaltGuestError, HaltGuestErrorKind, Phase},
    guest::SERIAL_PORT,
};

/// Upper bound on captured serial bytes before the proof fails closed.
pub(crate) const SERIAL_CAPTURE_LIMIT: usize = 4096;

/// How the guest stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestExit {
    /// The guest executed `hlt` and KVM returned `KVM_EXIT_HLT`.
    Halt,
}

/// The vCPU thread's result: captured console bytes plus the terminal exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunOutcome {
    pub(crate) serial: Vec<u8>,
    pub(crate) exit: GuestExit,
}

/// Runs the vCPU until it halts, an unexpected exit occurs, or the watchdog interrupts it.
pub(crate) fn run(vcpu: &mut VcpuFd) -> Result<RunOutcome, HaltGuestError> {
    let mut serial = Vec::new();
    loop {
        match vcpu.run() {
            Ok(VcpuExit::IoOut(SERIAL_PORT, data)) => capture(&mut serial, data)?,
            Ok(VcpuExit::Hlt) => {
                return Ok(RunOutcome {
                    serial,
                    exit: GuestExit::Halt,
                });
            }
            Ok(VcpuExit::Intr) => {}
            Ok(exit) => {
                return Err(HaltGuestError::new(
                    Phase::Run,
                    HaltGuestErrorKind::UnexpectedExit(classify(&exit)),
                ));
            }
            Err(error) if error.errno() == libc::EINTR => {
                return Err(HaltGuestError::new(Phase::Run, HaltGuestErrorKind::Timeout));
            }
            Err(error) => return Err(HaltGuestError::os(Phase::Run, error)),
        }
    }
}

pub(crate) fn capture(serial: &mut Vec<u8>, data: &[u8]) -> Result<(), HaltGuestError> {
    let remaining = SERIAL_CAPTURE_LIMIT.saturating_sub(serial.len());
    if data.len() > remaining {
        return Err(HaltGuestError::invalid(
            Phase::Run,
            "serial capture exceeded 4 KiB before the guest halted",
        ));
    }
    serial.extend_from_slice(data);
    Ok(())
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
    fn capture_is_bounded() {
        let mut serial = Vec::new();
        capture(&mut serial, b"SOMA").unwrap();
        assert_eq!(serial, b"SOMA");
        let filler = vec![b'x'; SERIAL_CAPTURE_LIMIT - 4];
        capture(&mut serial, &filler).unwrap();
        assert_eq!(serial.len(), SERIAL_CAPTURE_LIMIT);
        assert!(capture(&mut serial, b"y").is_err());
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
