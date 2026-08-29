//! The single place that composes the SOMA-owned kernel command line.
//!
//! The fixed ordered set comes from the machine contract. SOMA appends only the arguments its
//! own initramfs and challenge need, so the complete line stays a function of the contract and
//! the boot inputs and can become part of `GenerationId` later. Callers never inject text.

use std::fmt::Write as _;

/// The fixed ordered diagnostic arguments from the `x86_64` machine contract.
pub(crate) const FIXED_ARGUMENTS: [&str; 9] = [
    "console=ttyS0",
    "reboot=k",
    "panic=1",
    "nomodule",
    "random.trust_cpu=off",
    "pci=off",
    "acpi=off",
    "noapic",
    "cryptomgr.notests",
];

/// Init path inside the SOMA initramfs fixture and the compiled Generation initramfs.
pub(crate) const INITRAMFS_INIT: &str = "rdinit=/init";
/// Root and overlay identification appended by the Generation compiler contract.
pub(crate) const GENERATION_DISKS: &str = "soma.lower=/dev/vda soma.upper=/dev/vdb";
/// The kernel argument that carries the challenge nonce to the guest.
pub(crate) const NONCE_ARGUMENT: &str = "soma.nonce";
/// The prefix of the challenge-bound sentinel the guest writes to its console.
pub(crate) const SENTINEL_PREFIX: &str = "SOMA-BOOT-";

/// A fresh 64-bit challenge that binds one boot's serial sentinel to one run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootNonce([u8; 8]);

impl BootNonce {
    /// Wraps caller-supplied fresh bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Lowercase hexadecimal encoding, sixteen characters.
    #[must_use]
    pub fn hex(&self) -> String {
        self.0
            .iter()
            .fold(String::with_capacity(16), |mut text, byte| {
                let _ = write!(text, "{byte:02x}");
                text
            })
    }

    /// The exact line the guest must write, without its trailing newline.
    #[must_use]
    pub fn sentinel(&self) -> String {
        format!("{SENTINEL_PREFIX}{}", self.hex())
    }
}

/// Composes the complete command line for one compiled Generation: the fixed contract set,
/// the five `virtio_mmio.device=` declarations from the bus table, the initramfs init, and
/// the root and overlay identification.
///
/// It must equal the command line bound into the Generation manifest byte for byte.
pub(crate) fn compose_generation() -> String {
    [
        FIXED_ARGUMENTS.join(" "),
        crate::virtio::kernel_command_line(),
        INITRAMFS_INIT.to_owned(),
        GENERATION_DISKS.to_owned(),
    ]
    .join(" ")
}

/// Composes the complete command line for one diagnostic boot.
pub(crate) fn compose(initramfs: bool, nonce: Option<&BootNonce>) -> String {
    let mut arguments: Vec<String> = FIXED_ARGUMENTS.iter().map(|s| (*s).to_owned()).collect();
    if initramfs {
        arguments.push(INITRAMFS_INIT.to_owned());
    }
    if let Some(nonce) = nonce {
        arguments.push(format!("{NONCE_ARGUMENT}={}", nonce.hex()));
    }
    arguments.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x86_64::boot_info::DIAGNOSTIC_CMDLINE;

    #[test]
    fn fixed_set_matches_the_contract_line() {
        assert_eq!(compose(false, None), DIAGNOSTIC_CMDLINE);
    }

    #[test]
    fn generation_line_is_contract_devices_init_and_disks() {
        let line = compose_generation();
        assert!(line.starts_with(DIAGNOSTIC_CMDLINE));
        assert!(line.contains(" virtio_mmio.device=4K@0xd0000000:5:0 "));
        assert!(line.contains(" virtio_mmio.device=4K@0xd0004000:9:4 "));
        assert!(line.ends_with(" rdinit=/init soma.lower=/dev/vda soma.upper=/dev/vdb"));
        assert_eq!(line.matches("virtio_mmio.device=").count(), 5);
        assert!(!line.contains("soma.nonce"));
    }

    #[test]
    fn appends_init_and_nonce_in_a_fixed_order() {
        let nonce = BootNonce::new([0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33]);
        assert_eq!(nonce.hex(), "deadbeef00112233");
        assert_eq!(nonce.sentinel(), "SOMA-BOOT-deadbeef00112233");
        assert_eq!(
            compose(true, Some(&nonce)),
            format!("{DIAGNOSTIC_CMDLINE} rdinit=/init soma.nonce=deadbeef00112233")
        );
        assert_eq!(
            compose(true, None),
            format!("{DIAGNOSTIC_CMDLINE} rdinit=/init")
        );
    }
}
