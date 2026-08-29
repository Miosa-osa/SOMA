//! Waiting for exactly the two contract virtio block devices to appear.

use std::collections::BTreeSet;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use super::{BootFailure, BootStep, OVERLAY_DEVICE, ROOT_DEVICE};

const EXPECTED_BLOCK_DEVICES: [&str; 2] = ["vda", "vdb"];
const DEVICE_POLL: Duration = Duration::from_millis(5);

pub(super) fn wait_for_devices(deadline: Instant) -> Result<(), BootFailure> {
    loop {
        let names = block_device_names().unwrap_or_default();
        if names == expected_block_devices()
            && is_block_device(ROOT_DEVICE)
            && is_block_device(OVERLAY_DEVICE)
        {
            return Ok(());
        }
        if names.len() > EXPECTED_BLOCK_DEVICES.len() || Instant::now() >= deadline {
            return Err(BootFailure {
                step: BootStep::Devices,
                errno: libc::ETIMEDOUT,
            });
        }
        thread::sleep(DEVICE_POLL);
    }
}

fn block_device_names() -> std::io::Result<BTreeSet<String>> {
    fs::read_dir("/sys/block")?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect()
}

fn expected_block_devices() -> BTreeSet<String> {
    EXPECTED_BLOCK_DEVICES
        .iter()
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
    fn the_device_contract_is_exactly_two_virtio_block_devices() {
        assert_eq!(ROOT_DEVICE, "/dev/vda");
        assert_eq!(OVERLAY_DEVICE, "/dev/vdb");
        assert_eq!(expected_block_devices().len(), 2);
        assert!(!is_block_device("/proc/self/exe"));
    }
}
