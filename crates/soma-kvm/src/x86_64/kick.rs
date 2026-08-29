//! Race-free interruption of a vCPU that is inside `KVM_RUN`.
//!
//! The vCPU thread blocks one real-time signal everywhere except inside `KVM_RUN`, where KVM
//! installs the run mask that unblocks it. A `pthread_kill` from the watchdog therefore either
//! interrupts an in-flight `KVM_RUN` with `EINTR` or stays pending until the next `KVM_RUN`
//! starts, which then returns `EINTR` immediately. No kick can be lost between iterations.

use std::{
    mem,
    os::{fd::AsRawFd, unix::thread::JoinHandleExt},
    ptr,
    thread::JoinHandle,
};

use kvm_bindings::KVMIO;
use kvm_ioctls::VcpuFd;
use libc::{SA_SIGINFO, c_int, c_void, sigaction, siginfo_t, sigset_t};

use super::error::{MachineError, MachineErrorKind, Phase};

const SIGNAL_OFFSET: c_int = 7;
const KERNEL_MASK_BYTES: usize = 8;
const IOC_WRITE: u64 = 1;
const IOC_NRBITS: u64 = 8;
const IOC_TYPEBITS: u64 = 8;
const IOC_SIZEBITS: u64 = 14;
const KVM_SET_SIGNAL_MASK_NR: u64 = 0x8b;

/// `_IOW(KVMIO, 0x8b, struct kvm_signal_mask)` where the fixed header is one `u32`.
pub(crate) const fn kvm_set_signal_mask_request() -> u64 {
    let header = mem::size_of::<u32>() as u64;
    (IOC_WRITE << (IOC_NRBITS + IOC_TYPEBITS + IOC_SIZEBITS))
        | (header << (IOC_NRBITS + IOC_TYPEBITS))
        | ((KVMIO as u64) << IOC_NRBITS)
        | KVM_SET_SIGNAL_MASK_NR
}

/// The real-time signal reserved for interrupting `KVM_RUN`.
pub(crate) fn signal_number() -> Result<c_int, MachineError> {
    let signal = libc::SIGRTMIN()
        .checked_add(SIGNAL_OFFSET)
        .ok_or_else(|| MachineError::invalid(Phase::Run, "watchdog signal overflow"))?;
    let mask_bits = c_int::try_from(KERNEL_MASK_BYTES * 8)
        .map_err(|_| MachineError::invalid(Phase::Run, "signal mask width overflow"))?;
    if signal > libc::SIGRTMAX() || signal > mask_bits {
        return Err(MachineError::invalid(
            Phase::Run,
            "no KVM-compatible real-time signal is available",
        ));
    }
    Ok(signal)
}

/// Installs a no-op process-wide handler so the signal interrupts `KVM_RUN` instead of killing.
pub(crate) struct HandlerGuard {
    signal: c_int,
    previous: sigaction,
}

impl HandlerGuard {
    #[allow(unsafe_code)]
    pub(crate) fn install(signal: c_int) -> Result<Self, MachineError> {
        // SAFETY: `action` and `previous` are zero-initialized sigaction storage that libc
        // fully writes. The handler is async-signal-safe (it does nothing). Return values are
        // checked, and the previous action is restored in `Drop`.
        unsafe {
            let mut action: sigaction = mem::zeroed();
            action.sa_sigaction = noop_handler as *const () as usize;
            action.sa_flags = SA_SIGINFO;
            if libc::sigfillset(&raw mut action.sa_mask) != 0 {
                return Err(MachineError::last_os(Phase::Run));
            }
            let mut previous: sigaction = mem::zeroed();
            if libc::sigaction(signal, &raw const action, &raw mut previous) != 0 {
                return Err(MachineError::last_os(Phase::Run));
            }
            Ok(Self { signal, previous })
        }
    }
}

impl Drop for HandlerGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `previous` was written by sigaction for this signal and is still valid.
        let _ignored =
            unsafe { libc::sigaction(self.signal, &raw const self.previous, ptr::null_mut()) };
    }
}

/// Blocks the signal on the calling vCPU thread and unblocks it only inside `KVM_RUN`.
pub(crate) struct RunMaskGuard {
    original: sigset_t,
}

impl RunMaskGuard {
    #[allow(unsafe_code)]
    pub(crate) fn install(vcpu: &VcpuFd, signal: c_int) -> Result<Self, MachineError> {
        if mem::size_of::<sigset_t>() < KERNEL_MASK_BYTES {
            return Err(MachineError::invalid(
                Phase::Run,
                "pthread signal mask is narrower than the KVM signal mask",
            ));
        }
        // SAFETY: All sigset_t values are initialized by libc before use, the calls only affect
        // the calling thread's mask, every return is checked, and sigset_t is at least as wide
        // as the eight-byte prefix copied into the KVM run mask.
        unsafe {
            let mut original: sigset_t = mem::zeroed();
            if libc::pthread_sigmask(libc::SIG_BLOCK, ptr::null(), &raw mut original) != 0 {
                return Err(MachineError::last_os(Phase::Run));
            }
            let mut blocked = original;
            if libc::sigaddset(&raw mut blocked, signal) != 0 {
                return Err(MachineError::last_os(Phase::Run));
            }
            if libc::pthread_sigmask(libc::SIG_SETMASK, &raw const blocked, ptr::null_mut()) != 0 {
                return Err(MachineError::last_os(Phase::Run));
            }
            let guard = Self { original };
            let mut run_mask = [0_u8; KERNEL_MASK_BYTES];
            ptr::copy_nonoverlapping(
                (&raw const blocked).cast::<u8>(),
                run_mask.as_mut_ptr(),
                KERNEL_MASK_BYTES,
            );
            clear_bit(&mut run_mask, signal)?;
            set_kvm_signal_mask(vcpu, &run_mask)?;
            Ok(guard)
        }
    }
}

impl Drop for RunMaskGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `original` came from this thread's own mask and is restored on the same thread.
        let _ignored = unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &raw const self.original, ptr::null_mut())
        };
    }
}

/// Sends the reserved signal to the vCPU thread.
#[allow(unsafe_code)]
pub(crate) fn kick(worker: &JoinHandle<()>, signal: c_int) -> Result<(), MachineError> {
    // SAFETY: The unconsumed JoinHandle keeps the target pthread valid, and the process handler
    // for `signal` stays installed until after the thread is joined.
    let result = unsafe { libc::pthread_kill(worker.as_pthread_t(), signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(MachineError::new(Phase::Run, MachineErrorKind::Os(result)))
    }
}

pub(crate) fn clear_bit(
    mask: &mut [u8; KERNEL_MASK_BYTES],
    signal: c_int,
) -> Result<(), MachineError> {
    let bit = u32::try_from(signal.checked_sub(1).unwrap_or(-1))
        .map_err(|_| MachineError::invalid(Phase::Run, "invalid signal number"))?;
    let bit_mask = 1_u64
        .checked_shl(bit)
        .filter(|_| bit < 64)
        .ok_or_else(|| MachineError::invalid(Phase::Run, "signal outside KVM mask"))?;
    *mask = (u64::from_ne_bytes(*mask) & !bit_mask).to_ne_bytes();
    Ok(())
}

#[allow(unsafe_code)]
fn set_kvm_signal_mask(vcpu: &VcpuFd, mask: &[u8; KERNEL_MASK_BYTES]) -> Result<(), MachineError> {
    let mut payload = [0_u8; mem::size_of::<u32>() + KERNEL_MASK_BYTES];
    let len = u32::try_from(KERNEL_MASK_BYTES)
        .map_err(|_| MachineError::invalid(Phase::Run, "mask length overflow"))?;
    payload[..4].copy_from_slice(&len.to_ne_bytes());
    payload[4..].copy_from_slice(mask);
    // SAFETY: The calling thread exclusively owns `vcpu`. KVM reads the four-byte length header
    // and the following eight declared bytes from `payload`, which outlives the call. The
    // request number is the checked `_IOW` encoding, and the return value is inspected.
    let result = unsafe {
        libc::ioctl(
            vcpu.as_raw_fd(),
            kvm_set_signal_mask_request() as libc::c_ulong,
            payload.as_ptr().cast::<c_void>(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(MachineError::last_os(Phase::Run))
    }
}

extern "C" fn noop_handler(_: c_int, _: *mut siginfo_t, _: *mut c_void) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_the_linux_kvm_set_signal_mask_encoding() {
        assert_eq!(kvm_set_signal_mask_request(), 0x4004_ae8b);
    }

    #[test]
    fn clear_bit_removes_only_the_target_signal() {
        let mut mask = u64::MAX.to_ne_bytes();
        clear_bit(&mut mask, 9).unwrap();
        assert_eq!(u64::from_ne_bytes(mask), !(1_u64 << 8));
        assert!(clear_bit(&mut mask, 0).is_err());
        assert!(clear_bit(&mut mask, 65).is_err());
    }

    #[test]
    fn signal_number_is_a_real_time_signal_inside_the_kvm_mask() {
        let signal = signal_number().unwrap();
        assert!(signal >= libc::SIGRTMIN());
        assert!(signal <= libc::SIGRTMAX());
        assert!(signal <= 64);
    }
}
