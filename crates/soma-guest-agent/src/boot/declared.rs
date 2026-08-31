//! What the Generation told this guest it has, read from the kernel command line.
//!
//! The command line is composed by the machine layer from the same device set that decided
//! which devices to build, so it is the guest's copy of that decision rather than a second
//! opinion about it. A machine with no private overlay names no upper and the guest mounts its
//! root read-only; a machine with no network device names no interface and the guest performs
//! no network repair. Probing for the devices instead would make a device that failed to appear
//! indistinguishable from one that was never built.

use std::fs;

/// The command-line argument naming the private overlay head.
pub const UPPER_ARGUMENT: &str = "soma.upper=";
/// The command-line argument naming the network interface.
pub const NET_ARGUMENT: &str = "soma.net=";

const CMDLINE: &str = "/proc/cmdline";

/// The optional devices this machine was built with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Declared {
    /// Whether the machine has a private writable overlay to compose a writable root from.
    pub overlay: bool,
    /// Whether the machine has a network device to repair.
    pub net: bool,
}

impl Declared {
    /// Reads the declaration from the mounted procfs.
    ///
    /// An unreadable command line declares nothing, which is the smaller machine: a guest that
    /// then waits for an overlay it was never given would hang instead of failing.
    #[must_use]
    pub fn from_proc() -> Self {
        fs::read_to_string(CMDLINE).map_or_else(|_| Self::default(), |line| Self::parse(&line))
    }

    /// Parses one command line.
    #[must_use]
    pub fn parse(line: &str) -> Self {
        let mut declared = Self::default();
        for argument in line.split_ascii_whitespace() {
            declared.overlay |= argument.starts_with(UPPER_ARGUMENT);
            declared.net |= argument.starts_with(NET_ARGUMENT);
        }
        declared
    }

    /// How many virtio block devices this machine has.
    #[must_use]
    pub const fn block_devices(self) -> usize {
        if self.overlay { 2 } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_line_declares_both_optional_devices() {
        let declared = Declared::parse(
            "console=ttyS0 rdinit=/init soma.lower=/dev/vda soma.upper=/dev/vdb soma.net=eth0",
        );
        assert_eq!(
            declared,
            Declared {
                overlay: true,
                net: true
            }
        );
        assert_eq!(declared.block_devices(), 2);
    }

    #[test]
    fn a_read_only_line_declares_one_block_device_and_no_network() {
        let declared = Declared::parse("console=ttyS0 rdinit=/init soma.lower=/dev/vda");
        assert_eq!(
            declared,
            Declared {
                overlay: false,
                net: false
            }
        );
        assert_eq!(declared.block_devices(), 1);
    }

    #[test]
    fn a_prefix_of_an_argument_is_not_that_argument() {
        // `soma.lower=` must not be mistaken for `soma.upper=`, and no argument that merely
        // contains the text counts: only one that starts with it does.
        let declared = Declared::parse("soma.lower=/dev/vda notsoma.upper=/dev/vdb");
        assert!(!declared.overlay);
        assert!(!declared.net);
    }
}
