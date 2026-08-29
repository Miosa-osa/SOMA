use super::*;
use crate::virtio::device::test_device::{TEST_DEVICE_ID, TEST_FEATURE, TestDevice};
use crate::virtio::device::{ConfigAccessError, SOMA_VENDOR_ID};
use crate::virtio::guest_memory::VecGuestMemory;
use registers::*;
use violation::TransportViolationKind;

const W: AccessWidth = AccessWidth::U32;

pub(super) fn memory() -> VecGuestMemory {
    VecGuestMemory::flat(0x8000).expect("memory")
}

pub(super) fn transport() -> MmioTransport<TestDevice> {
    MmioTransport::new(TestDevice::default()).expect("transport")
}

pub(super) fn write(t: &mut MmioTransport<TestDevice>, offset: u64, value: u64) -> TransportEvent {
    t.write(offset, W, value, &memory())
        .unwrap_or_else(|violation| panic!("write {offset:#x}: {violation}"))
}

#[test]
fn golden_register_offsets_match_the_specification() {
    let golden = [
        (0x000, Register::MagicValue),
        (0x004, Register::Version),
        (0x008, Register::DeviceId),
        (0x00c, Register::VendorId),
        (0x010, Register::DeviceFeatures),
        (0x014, Register::DeviceFeaturesSel),
        (0x020, Register::DriverFeatures),
        (0x024, Register::DriverFeaturesSel),
        (0x030, Register::QueueSel),
        (0x034, Register::QueueNumMax),
        (0x038, Register::QueueNum),
        (0x044, Register::QueueReady),
        (0x050, Register::QueueNotify),
        (0x060, Register::InterruptStatus),
        (0x064, Register::InterruptAck),
        (0x070, Register::Status),
        (0x080, Register::QueueDescLow),
        (0x084, Register::QueueDescHigh),
        (0x090, Register::QueueDriverLow),
        (0x094, Register::QueueDriverHigh),
        (0x0a0, Register::QueueDeviceLow),
        (0x0a4, Register::QueueDeviceHigh),
        (0x0ac, Register::ShmSel),
        (0x0b0, Register::ShmLenLow),
        (0x0b4, Register::ShmLenHigh),
        (0x0b8, Register::ShmBaseLow),
        (0x0bc, Register::ShmBaseHigh),
        (0x0c0, Register::QueueReset),
        (0x0fc, Register::ConfigGeneration),
        (0x100, Register::Config(0)),
        (0xfff, Register::Config(0xeff)),
    ];
    for (offset, register) in golden {
        assert_eq!(Register::decode(offset), Some(register), "{offset:#x}");
    }
    for offset in [0x018, 0x03c, 0x040, 0x048, 0x0f8, 0x1000, u64::MAX] {
        assert_eq!(Register::decode(offset), None, "{offset:#x}");
    }
}

#[test]
fn identity_registers_report_modern_transport() {
    let mut t = transport();
    assert_eq!(t.read(REG_MAGIC_VALUE, W), Ok(0x7472_6976));
    assert_eq!(t.read(REG_VERSION, W), Ok(2));
    assert_eq!(t.read(REG_DEVICE_ID, W), Ok(u64::from(TEST_DEVICE_ID)));
    assert_eq!(t.read(REG_VENDOR_ID, W), Ok(u64::from(SOMA_VENDOR_ID)));
    assert_eq!(t.read(REG_SHM_LEN_LOW, W), Ok(0xffff_ffff));
    assert_eq!(t.read(REG_SHM_BASE_HIGH, W), Ok(0xffff_ffff));
    assert_eq!(t.read(REG_QUEUE_RESET, W), Ok(0));
    assert_eq!(t.read(REG_CONFIG_GENERATION, W), Ok(0));
    assert_eq!(t.read(REG_STATUS, W), Ok(0));
}

#[test]
fn device_features_follow_selector_and_unknown_selectors_read_zero() {
    let mut t = transport();
    assert_eq!(t.read(REG_DEVICE_FEATURES, W), Ok(TEST_FEATURE));
    write(&mut t, REG_DEVICE_FEATURES_SEL, 1);
    assert_eq!(t.read(REG_DEVICE_FEATURES, W), Ok(1));
    write(&mut t, REG_DEVICE_FEATURES_SEL, 2);
    assert_eq!(t.read(REG_DEVICE_FEATURES, W), Ok(0));
}

#[test]
fn rejects_wrong_width_write_only_reads_read_only_writes_and_unknown_offsets() {
    let mut t = transport();
    assert_eq!(
        t.read(REG_STATUS, AccessWidth::U8),
        Err(TransportViolation::WidthMismatch { offset: REG_STATUS })
    );
    assert_eq!(
        t.read(REG_QUEUE_NOTIFY, W),
        Err(TransportViolation::ReadOfWriteOnly {
            offset: REG_QUEUE_NOTIFY
        })
    );
    assert_eq!(
        t.write(REG_MAGIC_VALUE, W, 0, &memory()),
        Err(TransportViolation::WriteOfReadOnly {
            offset: REG_MAGIC_VALUE
        })
    );
    assert_eq!(
        t.write(0x018, W, 0, &memory()),
        Err(TransportViolation::UnknownRegister { offset: 0x018 })
    );
    assert_eq!(
        t.write(REG_QUEUE_SEL, W, 1 << 32, &memory()),
        Err(TransportViolation::WidthMismatch {
            offset: REG_QUEUE_SEL
        })
    );
    assert_eq!(
        t.write(REG_QUEUE_RESET, W, 1, &memory()),
        Err(TransportViolation::RingResetUnsupported)
    );
    assert_eq!(
        t.violations().count(TransportViolationKind::WidthMismatch),
        2
    );
    assert_eq!(t.violations().total(), 5);
}

#[test]
fn interrupt_ack_clears_only_acknowledged_known_bits() {
    let mut t = transport();
    t.signal_config_change();
    assert_eq!(t.read(REG_INTERRUPT_STATUS, W), Ok(2));
    assert_eq!(t.read(REG_CONFIG_GENERATION, W), Ok(1));
    write(&mut t, REG_INTERRUPT_ACK, 1);
    assert_eq!(t.interrupt_status(), 2, "unrelated bit survives");
    assert_eq!(
        t.write(REG_INTERRUPT_ACK, W, 0x10, &memory()),
        Err(TransportViolation::InterruptAckUnknownBits { value: 0x10 })
    );
    assert_eq!(t.interrupt_status(), 2);
    write(&mut t, REG_INTERRUPT_ACK, 2);
    assert_eq!(t.interrupt_status(), 0);
}

#[test]
fn config_space_is_bounded_and_delegated_with_width() {
    let mut t = transport();
    t.device_mut().config = [1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(t.read(0x100, AccessWidth::U8), Ok(1));
    assert_eq!(t.read(0x100, AccessWidth::U16), Ok(0x0201));
    assert_eq!(t.read(0x104, W), Ok(0x0807_0605));
    assert_eq!(t.read(0x100, AccessWidth::U64), Ok(0x0807_0605_0403_0201));
    assert_eq!(
        t.read(0x105, W),
        Err(TransportViolation::ConfigOutOfBounds { offset: 5 })
    );
    assert_eq!(
        t.read(0xfff, AccessWidth::U8),
        Err(TransportViolation::ConfigOutOfBounds { offset: 0xeff })
    );
    assert_eq!(
        t.write(0x100, AccessWidth::U8, 9, &memory()),
        Err(TransportViolation::ConfigAccess(
            ConfigAccessError::ReadOnly
        ))
    );
    assert_eq!(
        t.write(0x106, W, 0xbbaa, &memory()),
        Err(TransportViolation::ConfigOutOfBounds { offset: 6 })
    );
    assert_eq!(
        t.write(0x106, AccessWidth::U16, 0xbbaa, &memory()),
        Ok(TransportEvent::ConfigWritten { offset: 6, len: 2 })
    );
    assert_eq!(t.device().config, [1, 2, 3, 4, 5, 6, 0xaa, 0xbb]);
}
