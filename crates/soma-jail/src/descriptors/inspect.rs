//! `fstat`-based inspection of one descriptor slot.
//!
//! Only `fstat` and, before seccomp, `getsockopt` are used so the checks work after procfs is
//! gone and inside the startup filter.

#![allow(unsafe_code)]

use std::io;

use super::{DescriptorError, KVM_DEVICE, TAP_DEVICE, VerificationDepth};
use crate::manifest::{DescriptorKind, DescriptorRole};

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

pub(super) fn kind_of(mode: u32) -> Option<DescriptorKind> {
    match mode & libc::S_IFMT {
        0 => Some(DescriptorKind::AnonInode),
        libc::S_IFCHR => Some(DescriptorKind::CharDevice),
        libc::S_IFREG => Some(DescriptorKind::RegularFile),
        libc::S_IFSOCK => Some(DescriptorKind::Socket),
        libc::S_IFIFO => Some(DescriptorKind::Fifo),
        _ => None,
    }
}

/// Decodes `st_rdev` exactly as glibc's `gnu_dev_major` and `gnu_dev_minor` do.
pub(super) fn device_numbers(rdev: u64) -> (u32, u32) {
    let word = |value: u64| u32::try_from(value & u64::from(u32::MAX)).unwrap_or(u32::MAX);
    let major = word((rdev >> 8) & 0xfff) | (word(rdev >> 32) & !0xfff);
    let minor = word(rdev & 0xff) | (word(rdev >> 12) & !0xff);
    (major, minor)
}

/// `fstat` one slot; `Err(errno)` when it is closed or unreadable.
pub(super) fn stat_slot(slot: u32) -> Result<libc::stat, i32> {
    let fd = libc::c_int::try_from(slot).map_err(|_| libc::EBADF)?;
    // SAFETY: `stat` is zeroed storage the kernel fills completely on success.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is any integer and `stat` is valid writable storage for the whole call.
    if unsafe { libc::fstat(fd, &raw mut stat) } < 0 {
        return Err(errno());
    }
    Ok(stat)
}

pub(super) fn expect_kind(
    slot: u32,
    kinds: &[DescriptorKind],
) -> Result<libc::stat, DescriptorError> {
    let stat = stat_slot(slot).map_err(|errno| DescriptorError::Missing { slot, errno })?;
    let found = kind_of(stat.st_mode);
    if !found.is_some_and(|kind| kinds.contains(&kind)) {
        return Err(DescriptorError::Kind { slot, found });
    }
    Ok(stat)
}

fn expect_seqpacket(slot: u32) -> Result<(), DescriptorError> {
    let mut socket_type: libc::c_int = 0;
    let mut length = libc::socklen_t::try_from(size_of::<libc::c_int>()).unwrap_or(4);
    // SAFETY: `socket_type` and `length` are valid writable storage sized for `SO_TYPE`.
    let result = unsafe {
        libc::getsockopt(
            libc::c_int::try_from(slot).unwrap_or(-1),
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&raw mut socket_type).cast(),
            &raw mut length,
        )
    };
    if result != 0 || socket_type != libc::SOCK_SEQPACKET {
        return Err(DescriptorError::NotSeqpacket { slot });
    }
    Ok(())
}

/// Checks that `slot` holds a descriptor acceptable for `role`.
pub(super) fn expect_slot(
    slot: u32,
    role: DescriptorRole,
    depth: VerificationDepth,
) -> Result<(), DescriptorError> {
    let stat = expect_kind(slot, role.expected_kinds())?;
    let expected_device = match role {
        DescriptorRole::Kvm => Some(KVM_DEVICE),
        DescriptorRole::Tap => Some(TAP_DEVICE),
        _ => None,
    };
    if expected_device.is_some_and(|device| device_numbers(stat.st_rdev) != device) {
        return Err(DescriptorError::Device { slot });
    }
    if role == DescriptorRole::Control && depth == VerificationDepth::Launcher {
        expect_seqpacket(slot)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbers_decode_kvm_and_tun() {
        assert_eq!(device_numbers(0x0a_e8), KVM_DEVICE);
        assert_eq!(device_numbers(0x0a_c8), TAP_DEVICE);
        assert_eq!(
            device_numbers((0x1000 << 32) | (0x100 << 12)),
            (0x1000, 0x100)
        );
        assert_eq!(device_numbers(0x0103), (1, 3));
    }

    #[test]
    fn kinds_follow_st_mode() {
        assert_eq!(kind_of(0o600), Some(DescriptorKind::AnonInode));
        assert_eq!(
            kind_of(libc::S_IFCHR | 0o600),
            Some(DescriptorKind::CharDevice)
        );
        assert_eq!(kind_of(libc::S_IFSOCK), Some(DescriptorKind::Socket));
        assert_eq!(kind_of(libc::S_IFDIR), None);
    }

    #[test]
    fn a_closed_slot_reports_ebadf() {
        assert_eq!(stat_slot(u32::MAX).unwrap_err(), libc::EBADF);
        assert!(stat_slot(1).is_ok());
    }
}
