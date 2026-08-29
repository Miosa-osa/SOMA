//! Inputs for one kernel-boot proof.

use std::{path::PathBuf, time::Duration};

use crate::x86_64::BootNonce;

/// Inputs for one kernel-boot proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootKernelConfig {
    pub(super) kernel: PathBuf,
    pub(super) initramfs: Option<PathBuf>,
    pub(super) ram_bytes: u64,
    pub(super) timeout: Duration,
    pub(super) nonce: Option<BootNonce>,
    pub(super) stop_on_sentinel: bool,
    pub(super) pit: bool,
}

impl BootKernelConfig {
    /// Boots the ELF at `kernel` with `ram_bytes` of RAM under a run deadline of `timeout`.
    #[must_use]
    pub const fn new(kernel: PathBuf, ram_bytes: u64, timeout: Duration) -> Self {
        Self {
            kernel,
            initramfs: None,
            ram_bytes,
            timeout,
            nonce: None,
            stop_on_sentinel: false,
            pit: true,
        }
    }

    /// Attaches a `newc` cpio initramfs as the sole PVH module.
    #[must_use]
    pub fn with_initramfs(mut self, initramfs: PathBuf) -> Self {
        self.initramfs = Some(initramfs);
        self
    }

    /// Passes `nonce` on the command line and expects its sentinel on the console.
    #[must_use]
    pub const fn with_nonce(mut self, nonce: BootNonce) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// Stops the vCPU as soon as the sentinel arrives instead of waiting for an orderly exit.
    #[must_use]
    pub const fn stop_on_sentinel(mut self, stop: bool) -> Self {
        self.stop_on_sentinel = stop;
        self
    }

    /// Selects whether KVM's in-kernel programmable interval timer is created.
    #[must_use]
    pub const fn with_pit(mut self, pit: bool) -> Self {
        self.pit = pit;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_records_every_option() {
        let nonce = BootNonce::new([7; 8]);
        let config = BootKernelConfig::new(PathBuf::from("k"), 1, Duration::from_secs(1))
            .with_initramfs(PathBuf::from("i"))
            .with_nonce(nonce)
            .stop_on_sentinel(true)
            .with_pit(false);
        assert_eq!(config.initramfs, Some(PathBuf::from("i")));
        assert_eq!(config.nonce, Some(nonce));
        assert!(config.stop_on_sentinel);
        assert!(!config.pit);
    }
}
