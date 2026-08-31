//! Waiting for exactly the virtio block devices this Generation declared.

use std::collections::BTreeSet;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use super::{BootFailure, BootStep, Declared, OVERLAY_DEVICE, ROOT_DEVICE};

const DEVICE_POLL: Duration = Duration::from_millis(5);

/// Waits until the declared block devices, and no others, exist.
///
/// The count comes from the declaration rather than from a constant, so a machine built with no
/// private head stops waiting as soon as its single root device appears instead of timing out
/// on a second device nobody built, and a machine that was given one still refuses to boot
/// until it has both.
pub(super) fn wait_for_devices(deadline: Instant, declared: Declared) -> Result<(), BootFailure> {
    let expected = expected_block_devices(declared);
    loop {
        let names = block_device_names().unwrap_or_default();
        if names == expected && present(declared) {
            return Ok(());
        }
        if names.len() > expected.len() || Instant::now() >= deadline {
            return Err(BootFailure {
                step: BootStep::Devices,
                errno: libc::ETIMEDOUT,
            });
        }
        thread::sleep(DEVICE_POLL);
    }
}

/// Whether every declared device node is a block device.
fn present(declared: Declared) -> bool {
    is_block_device(ROOT_DEVICE) && (!declared.overlay || is_block_device(OVERLAY_DEVICE))
}

fn block_device_names() -> std::io::Result<BTreeSet<String>> {
    fs::read_dir("/sys/block")?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect()
}

fn expected_block_devices(declared: Declared) -> BTreeSet<String> {
    ["vda", "vdb"]
        .iter()
        .take(declared.block_devices())
        .map(|name| (*name).to_owned())
        .collect()
}

fn is_block_device(path: &str) -> bool {
    use std::os::unix::fs::FileTypeExt;
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_block_device())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declared_set_decides_how_many_block_devices_are_expected() {
        assert_eq!(ROOT_DEVICE, "/dev/vda");
        assert_eq!(OVERLAY_DEVICE, "/dev/vdb");
        let writable = Declared {
            overlay: true,
            net: true,
        };
        assert_eq!(expected_block_devices(writable).len(), 2);
        assert_eq!(expected_block_devices(Declared::default()).len(), 1);
        assert!(
            expected_block_devices(Declared::default()).contains("vda"),
            "the immutable root is never optional"
        );
        assert!(!is_block_device("/proc/self/exe"));
    }
}
