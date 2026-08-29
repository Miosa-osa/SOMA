//! A tiny echo device used to exercise the transport and queues end to end.

use super::{ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice};

/// Device-specific feature bit the test device offers.
pub(crate) const TEST_FEATURE: u64 = 1;
pub(crate) const TEST_DEVICE_ID: u32 = 0x7e;
pub(crate) const TEST_QUEUE_MAX: [u16; 2] = [64, 16];
pub(crate) const TEST_CONFIG_LEN: usize = 8;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TestDevice {
    pub(crate) config: [u8; TEST_CONFIG_LEN],
    pub(crate) activated_with: Option<u64>,
    pub(crate) resets: u32,
    pub(crate) reject_activation: bool,
}

impl VirtioDevice for TestDevice {
    fn device_id(&self) -> u32 {
        TEST_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        VIRTIO_F_VERSION_1 | TEST_FEATURE
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &TEST_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        TEST_CONFIG_LEN
    }

    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        let end = offset
            .checked_add(buf.len())
            .filter(|end| *end <= TEST_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        buf.copy_from_slice(&self.config[offset..end]);
        Ok(())
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) -> Result<(), ConfigAccessError> {
        let end = offset
            .checked_add(data.len())
            .filter(|end| *end <= TEST_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        if offset < 4 {
            return Err(ConfigAccessError::ReadOnly);
        }
        self.config[offset..end].copy_from_slice(data);
        Ok(())
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if self.reject_activation {
            return Err(ActivateError::BackendUnavailable);
        }
        self.activated_with = Some(negotiated_features);
        Ok(())
    }

    fn reset(&mut self) {
        self.activated_with = None;
        self.resets += 1;
    }

    fn snapshot_state(&self) -> Vec<u8> {
        self.config.to_vec()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        let config: [u8; TEST_CONFIG_LEN] =
            bytes.try_into().map_err(|_| DeviceStateError::Malformed)?;
        self.config = config;
        Ok(())
    }
}
