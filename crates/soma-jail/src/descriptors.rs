//! Sealing the child's descriptor table and verifying it from both sides.
//!
//! The parent computes a [`SealPlan`] whose sources already sit above every target slot, so
//! the child only performs `dup3` into fixed slots and one `close_range`.
//! Verification uses `fstat` alone so it works after procfs is gone, and it scans every slot
//! up to the descriptor limit so an injected descriptor cannot hide.

#![allow(unsafe_code)]

mod inspect;

use std::{
    error::Error,
    fmt, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

use self::inspect::{expect_kind, expect_slot, stat_slot};
use crate::manifest::{DescriptorKind, DescriptorManifest, DescriptorRole, STANDARD_STREAMS};

/// `/dev/kvm` is character device 10:232.
pub const KVM_DEVICE: (u32, u32) = (10, 232);
/// `/dev/net/tun` is character device 10:200.
pub const TAP_DEVICE: (u32, u32) = (10, 200);

/// How deep verification may look.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationDepth {
    /// Before seccomp: may use `getsockopt`; the executable and report slots must be open.
    Launcher,
    /// After `execveat`: `fstat` only; the executable and report slots must be closed.
    Sealed,
}

/// The close-on-exec pipe through which the pre-exec child reports a failed step.
#[must_use]
pub fn report_slot(manifest: &DescriptorManifest) -> u32 {
    manifest.executable_slot() + 1
}

/// The first slot the pre-exec child closes; everything below is accounted for.
#[must_use]
pub fn launcher_sealed_len(manifest: &DescriptorManifest) -> u32 {
    report_slot(manifest) + 1
}

/// Typed descriptor failure; every variant is `Copy` so the pre-exec child can report it
/// without allocating.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    Missing {
        slot: u32,
        errno: i32,
    },
    Kind {
        slot: u32,
        found: Option<DescriptorKind>,
    },
    Device {
        slot: u32,
    },
    NotSeqpacket {
        slot: u32,
    },
    /// An open descriptor exists beyond the sealed range.
    Unexpected {
        slot: u32,
    },
    Dup {
        slot: u32,
        errno: i32,
    },
    CloseRange(i32),
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { slot, errno } => {
                write!(formatter, "slot {slot} is closed (errno {errno})")
            }
            Self::Kind { slot, found } => write!(formatter, "slot {slot} has kind {found:?}"),
            Self::Device { slot } => write!(formatter, "slot {slot} is the wrong character device"),
            Self::NotSeqpacket { slot } => write!(formatter, "slot {slot} is not SOCK_SEQPACKET"),
            Self::Unexpected { slot } => {
                write!(formatter, "slot {slot} is open but not in the manifest")
            }
            Self::Dup { slot, errno } => {
                write!(formatter, "dup3 into slot {slot} failed (errno {errno})")
            }
            Self::CloseRange(errno) => write!(formatter, "close_range failed (errno {errno})"),
        }
    }
}

impl Error for DescriptorError {}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// One `dup3` the child performs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SealEntry {
    pub source: RawFd,
    pub slot: u32,
    pub cloexec: bool,
}

/// The child's complete descriptor plan; `sealed_len` is the first slot that must be closed.
///
/// The plan owns the relocated sources; the parent must drop it right after the clone so no
/// pipe or socket end stays open on the launcher side.
#[derive(Debug)]
pub struct SealPlan {
    entries: Vec<SealEntry>,
    sealed_len: u32,
    /// Keeps the relocated sources alive until the child has executed.
    _holders: Vec<OwnedFd>,
}

impl SealPlan {
    /// Relocates every source above `sealed_len` so no `dup3` target can overwrite a source.
    ///
    /// `sources` are `(slot, descriptor, cloexec)` in any order.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorError::Dup`] if `F_DUPFD_CLOEXEC` fails.
    pub fn new(
        sealed_len: u32,
        sources: Vec<(u32, &OwnedFd, bool)>,
    ) -> Result<Self, DescriptorError> {
        let floor = libc::c_int::try_from(sealed_len).map_err(|_| DescriptorError::Dup {
            slot: sealed_len,
            errno: libc::EINVAL,
        })?;
        let mut entries = Vec::with_capacity(sources.len());
        let mut holders = Vec::with_capacity(sources.len());
        for (slot, descriptor, cloexec) in sources {
            // SAFETY: `F_DUPFD_CLOEXEC` duplicates an open descriptor we own into the lowest
            // free number at or above `floor`; it touches no memory.
            let relocated =
                unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_DUPFD_CLOEXEC, floor) };
            if relocated < 0 {
                return Err(DescriptorError::Dup {
                    slot,
                    errno: errno(),
                });
            }
            // SAFETY: `relocated` is a fresh descriptor returned by the kernel that nothing else
            // owns.
            holders.push(unsafe { OwnedFd::from_raw_fd(relocated) });
            entries.push(SealEntry {
                source: relocated,
                slot,
                cloexec,
            });
        }
        Ok(Self {
            entries,
            sealed_len,
            _holders: holders,
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[SealEntry] {
        &self.entries
    }

    /// Performs every `dup3` and closes everything from `sealed_len` upward.
    ///
    /// Allocation-free: safe to call in a freshly cloned child of a multithreaded parent.
    ///
    /// # Errors
    ///
    /// Returns the first failing step.
    pub fn apply_in_child(&self) -> Result<(), DescriptorError> {
        for entry in &self.entries {
            let target = libc::c_int::try_from(entry.slot).map_err(|_| DescriptorError::Dup {
                slot: entry.slot,
                errno: libc::EINVAL,
            })?;
            let flags = if entry.cloexec { libc::O_CLOEXEC } else { 0 };
            // SAFETY: `dup3` takes two descriptors and a flag word and touches no memory.
            if unsafe { libc::dup3(entry.source, target, flags) } < 0 {
                return Err(DescriptorError::Dup {
                    slot: entry.slot,
                    errno: errno(),
                });
            }
        }
        // SAFETY: `close_range` takes integer bounds and a flag word and touches no memory; the
        // raw syscall is used because musl's libc binding does not expose the wrapper.
        let closed =
            unsafe { libc::syscall(libc::SYS_close_range, self.sealed_len, libc::c_uint::MAX, 0) };
        if closed < 0 {
            return Err(DescriptorError::CloseRange(errno()));
        }
        Ok(())
    }
}

/// Verifies the standard streams, every manifest slot, the executable and report slots, and
/// the absence of any other descriptor below `limit`.
///
/// Returns the number of descriptors that must be open at `depth`.
///
/// # Errors
///
/// Returns the first [`DescriptorError`] in slot order.
pub fn verify_sealed_table(
    manifest: &DescriptorManifest,
    depth: VerificationDepth,
    limit: u32,
) -> Result<u32, DescriptorError> {
    expect_slot(0, DescriptorRole::Log, depth)?;
    expect_slot(1, DescriptorRole::Log, depth)?;
    expect_slot(2, DescriptorRole::Log, depth)?;
    for (index, role) in manifest.roles().iter().enumerate() {
        expect_slot(manifest.slot_of(index), *role, depth)?;
    }
    let executable = manifest.executable_slot();
    let mut open = STANDARD_STREAMS + u32::try_from(manifest.roles().len()).unwrap_or(u32::MAX);
    let first_closed = match depth {
        VerificationDepth::Launcher => {
            expect_kind(executable, &[DescriptorKind::RegularFile])?;
            expect_kind(report_slot(manifest), &[DescriptorKind::Fifo])?;
            open += 2;
            launcher_sealed_len(manifest)
        }
        VerificationDepth::Sealed => executable,
    };
    for slot in first_closed..limit {
        match stat_slot(slot) {
            Err(errno) if errno == libc::EBADF => {}
            _ => return Err(DescriptorError::Unexpected { slot }),
        }
    }
    Ok(open)
}
