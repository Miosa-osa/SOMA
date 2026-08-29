use std::{io, mem, os::unix::thread::JoinHandleExt, ptr, thread::JoinHandle};

use kvm_bindings::{KVMIO, kvm_signal_mask};
use kvm_ioctls::VcpuFd;
use libc::{SA_SIGINFO, c_int, c_void, sigaction, sigfillset, siginfo_t, sigset_t};
use vmm_sys_util::{
    ioctl::ioctl_with_ptr,
    ioctl_iow_nr,
    signal::{SIGRTMAX, SIGRTMIN},
};

use super::super::Arm64BootError;

ioctl_iow_nr!(KVM_SET_SIGNAL_MASK, KVMIO, 0x8b, kvm_signal_mask);

const SIGNAL_OFFSET: c_int = 7;
const KERNEL_MASK_BYTES: usize = 8;
const KERNEL_MASK_LEN: u32 = 8;
const HEADER_BYTES: usize = mem::size_of::<u32>();
const PAYLOAD_BYTES: usize = HEADER_BYTES + KERNEL_MASK_BYTES;

pub(super) struct ProcessSignalGuard {
    signal: c_int,
    previous: sigaction,
    installed: bool,
}

impl ProcessSignalGuard {
    #[allow(unsafe_code)]
    pub(super) fn install() -> Result<Self, Arm64BootError> {
        let signal = signal_number()?;
        // SAFETY: Values are initialized sigaction storage. The no-op handler is async-signal-safe,
        // its mask is initialized, and every process-wide libc return is checked.
        unsafe {
            let mut action: sigaction = mem::zeroed();
            action.sa_sigaction = kick_handler as *const () as usize;
            action.sa_flags = SA_SIGINFO;
            if sigfillset(&raw mut action.sa_mask) != 0 {
                return Err(last_error("initialize vCPU watchdog signal action"));
            }
            let mut previous: sigaction = mem::zeroed();
            if libc::sigaction(signal, &raw const action, &raw mut previous) != 0 {
                return Err(last_error("install vCPU watchdog signal handler"));
            }
            Ok(Self {
                signal,
                previous,
                installed: true,
            })
        }
    }

    pub(super) const fn number(&self) -> c_int {
        self.signal
    }

    pub(super) fn restore(mut self) -> Result<(), Arm64BootError> {
        self.restore_action()?;
        self.installed = false;
        Ok(())
    }

    #[allow(unsafe_code)]
    fn restore_action(&self) -> Result<(), Arm64BootError> {
        // SAFETY: `previous` came from sigaction for this signal and remains initialized.
        let result =
            unsafe { libc::sigaction(self.signal, &raw const self.previous, ptr::null_mut()) };
        if result == 0 {
            Ok(())
        } else {
            Err(last_error("restore vCPU watchdog signal handler"))
        }
    }
}

impl Drop for ProcessSignalGuard {
    fn drop(&mut self) {
        if self.installed {
            let _ignored = self.restore_action();
        }
    }
}

pub(super) struct WorkerMaskGuard {
    original: sigset_t,
    active: bool,
}

impl WorkerMaskGuard {
    #[allow(unsafe_code)]
    pub(super) fn install(vcpu: &VcpuFd, signal: c_int) -> Result<Self, Arm64BootError> {
        if mem::size_of::<sigset_t>() < KERNEL_MASK_BYTES {
            return Err(Arm64BootError::message(
                "pthread signal mask is smaller than the Linux KVM signal mask",
            ));
        }
        // SAFETY: Values are initialized sigset_t storage. Calls affect only this worker's mask,
        // returns are checked, and sigset_t is larger than the copied eight-byte KVM ABI prefix.
        unsafe {
            let mut original = mem::zeroed();
            check_pthread(
                libc::pthread_sigmask(libc::SIG_BLOCK, ptr::null(), &raw mut original),
                "read vCPU worker signal mask",
            )?;
            let mut blocked = original;
            if libc::sigaddset(&raw mut blocked, signal) != 0 {
                return Err(last_error("add vCPU watchdog signal to worker mask"));
            }
            check_pthread(
                libc::pthread_sigmask(libc::SIG_SETMASK, &raw const blocked, ptr::null_mut()),
                "block vCPU watchdog signal outside KVM_RUN",
            )?;
            let guard = Self {
                original,
                active: true,
            };
            let mut run_mask = [0_u8; KERNEL_MASK_BYTES];
            ptr::copy_nonoverlapping(
                (&raw const blocked).cast::<u8>(),
                run_mask.as_mut_ptr(),
                KERNEL_MASK_BYTES,
            );
            clear_signal(&mut run_mask, signal)
                .map_err(|error| Arm64BootError::at("build KVM_RUN signal mask", error))?;
            set_kvm_signal_mask(vcpu, &run_mask)?;
            Ok(guard)
        }
    }

    pub(super) fn restore(mut self) -> Result<(), Arm64BootError> {
        self.restore_mask()?;
        self.active = false;
        Ok(())
    }

    #[allow(unsafe_code)]
    fn restore_mask(&self) -> Result<(), Arm64BootError> {
        // SAFETY: `original` came from this worker and is restored by that worker while live.
        let result = unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &raw const self.original, ptr::null_mut())
        };
        check_pthread(result, "restore vCPU worker signal mask")
    }
}

impl Drop for WorkerMaskGuard {
    fn drop(&mut self) {
        if self.active {
            let _ignored = self.restore_mask();
        }
    }
}

#[allow(unsafe_code)]
pub(super) fn kick(worker: &JoinHandle<()>, signal: c_int) -> Result<(), io::Error> {
    // SAFETY: The unconsumed JoinHandle keeps this pthread valid until the bounded join. `signal`
    // is validated as real-time and its process handler remains installed until after that join.
    let result = unsafe { libc::pthread_kill(worker.as_pthread_t(), signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

fn signal_number() -> Result<c_int, Arm64BootError> {
    let signal = SIGRTMIN()
        .checked_add(SIGNAL_OFFSET)
        .ok_or_else(|| Arm64BootError::message("vCPU watchdog signal number overflow"))?;
    let max_kvm_signal = i32::try_from(KERNEL_MASK_BYTES * 8)
        .map_err(|error| Arm64BootError::at("convert KVM signal-mask size", error))?;
    if signal > SIGRTMAX() || signal > max_kvm_signal {
        return Err(Arm64BootError::message(
            "no KVM-compatible real-time signal is available for the vCPU watchdog",
        ));
    }
    Ok(signal)
}

fn clear_signal(mask: &mut [u8; KERNEL_MASK_BYTES], signal: c_int) -> Result<(), io::Error> {
    let bit = signal
        .checked_sub(1)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let shift = u32::try_from(bit).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let bit_mask = 1_u64
        .checked_shl(shift)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    *mask = (u64::from_ne_bytes(*mask) & !bit_mask).to_ne_bytes();
    Ok(())
}

fn signal_mask_payload(mask: [u8; KERNEL_MASK_BYTES]) -> [u8; PAYLOAD_BYTES] {
    let mut payload = [0_u8; PAYLOAD_BYTES];
    payload[..HEADER_BYTES].copy_from_slice(&KERNEL_MASK_LEN.to_ne_bytes());
    payload[HEADER_BYTES..].copy_from_slice(&mask);
    payload
}

#[allow(unsafe_code)]
fn set_kvm_signal_mask(
    vcpu: &VcpuFd,
    mask: &[u8; KERNEL_MASK_BYTES],
) -> Result<(), Arm64BootError> {
    let payload = signal_mask_payload(*mask);
    // SAFETY: This worker solely owns the VcpuFd and its kvm_run mapping. KVM reads the four-byte
    // header and following declared eight bytes while `payload` remains live. The return is checked.
    let result = unsafe { ioctl_with_ptr(vcpu, KVM_SET_SIGNAL_MASK(), payload.as_ptr()) };
    if result == 0 {
        Ok(())
    } else {
        Err(last_error("install KVM_RUN signal mask"))
    }
}

fn check_pthread(result: c_int, stage: &str) -> Result<(), Arm64BootError> {
    if result == 0 {
        Ok(())
    } else {
        Err(Arm64BootError::at(
            stage,
            io::Error::from_raw_os_error(result),
        ))
    }
}

fn last_error(stage: &str) -> Arm64BootError {
    Arm64BootError::at(stage, io::Error::last_os_error())
}

extern "C" fn kick_handler(_: c_int, _: *mut siginfo_t, _: *mut c_void) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_mask_removes_only_the_target_signal() {
        let original = u64::MAX;
        let mut mask = original.to_ne_bytes();
        clear_signal(&mut mask, 9).unwrap();
        assert_eq!(u64::from_ne_bytes(mask), original & !(1_u64 << 8));
    }

    #[test]
    fn payload_uses_the_eight_byte_linux_kvm_mask() {
        let mask = [0x5a; KERNEL_MASK_BYTES];
        let payload = signal_mask_payload(mask);
        assert_eq!(mem::size_of::<kvm_signal_mask>(), HEADER_BYTES);
        assert_eq!(
            u32::from_ne_bytes(payload[..HEADER_BYTES].try_into().unwrap()),
            8
        );
        assert_eq!(&payload[HEADER_BYTES..], &mask);
    }
}
