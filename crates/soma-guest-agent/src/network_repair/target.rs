//! Capability detection for the one target ABI whose interface-request layouts are verified.
//!
//! `encoding.rs` hand-encodes the classic `ifreq` and `rtentry` byte layouts, and the request
//! codes and C integer widths those layouts assume hold on Linux `x86_64` only.
//! Handing an `x86_64`-shaped `rtentry` to an ARM64 or 32-bit kernel would pass a differently
//! shaped structure to an unsafe `ioctl`, so repair compares the compiled ABI with the verified
//! ABI before it opens a socket and refuses every other target with a typed step.
//!
//! The `soma-guest-agent` binary is already gated to Linux `x86_64` in `main.rs`.
//! This second check is what makes widening that gate fail closed: a port that starts building
//! the agent for another architecture without verifying the layouts here refuses to install a
//! network identity instead of issuing the wrong `ioctl`, and the Instance is destroyed.

use super::{NetworkError, NetworkStep};

/// One target ABI identity, in the terms the kernel interface layouts depend on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TargetAbi {
    /// The `target_os` this binary was compiled for.
    pub(super) operating_system: &'static str,
    /// The `target_arch` this binary was compiled for.
    pub(super) architecture: &'static str,
    /// The width of a pointer and of `c_ulong` in bits.
    pub(super) pointer_width_bits: u32,
    /// The byte order of the multibyte fields inside the requests.
    pub(super) endian: &'static str,
}

/// The only ABI whose `ifreq` and `rtentry` encodings this crate has verified.
pub(super) const VERIFIED: TargetAbi = TargetAbi {
    operating_system: "linux",
    architecture: "x86_64",
    pointer_width_bits: 64,
    endian: "little",
};

/// The ABI this binary was actually compiled for.
pub(super) const COMPILED: TargetAbi = TargetAbi {
    operating_system: std::env::consts::OS,
    architecture: std::env::consts::ARCH,
    pointer_width_bits: usize::BITS,
    endian: if cfg!(target_endian = "little") {
        "little"
    } else {
        "big"
    },
};

/// Accepts only the verified ABI.
///
/// # Errors
///
/// Returns [`NetworkStep::UnsupportedTarget`] with `ENOTSUP` for every other ABI.
pub(super) fn require(compiled: TargetAbi) -> Result<(), NetworkError> {
    if compiled == VERIFIED {
        Ok(())
    } else {
        Err(NetworkError {
            step: NetworkStep::UnsupportedTarget,
            errno: libc::ENOTSUP,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsupported() -> NetworkError {
        NetworkError {
            step: NetworkStep::UnsupportedTarget,
            errno: libc::ENOTSUP,
        }
    }

    #[test]
    fn the_compiled_abi_is_the_verified_abi() {
        assert_eq!(COMPILED, VERIFIED);
        assert_eq!(require(COMPILED), Ok(()));
        assert_eq!(std::mem::size_of::<libc::c_ulong>() * 8, 64);
        assert_eq!(std::mem::size_of::<libc::c_short>() * 8, 16);
        assert_eq!(std::mem::size_of::<*mut libc::c_char>() * 8, 64);
    }

    #[test]
    fn every_unverified_abi_is_refused_before_a_socket_or_an_ioctl() {
        let others = [
            TargetAbi {
                architecture: "aarch64",
                ..VERIFIED
            },
            TargetAbi {
                architecture: "riscv64",
                ..VERIFIED
            },
            TargetAbi {
                architecture: "arm",
                pointer_width_bits: 32,
                ..VERIFIED
            },
            TargetAbi {
                architecture: "powerpc64",
                endian: "big",
                ..VERIFIED
            },
            TargetAbi {
                pointer_width_bits: 32,
                ..VERIFIED
            },
            TargetAbi {
                endian: "big",
                ..VERIFIED
            },
            TargetAbi {
                operating_system: "android",
                ..VERIFIED
            },
        ];

        for abi in others {
            assert_ne!(abi, VERIFIED);
            assert_eq!(require(abi), Err(unsupported()), "{abi:?} was accepted");
        }
    }
}
