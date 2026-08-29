//! Status lifecycle, feature negotiation, queue activation, and notification
//! gating through the driver-visible register interface.

use super::tests::{memory, transport, write};
use super::*;
use crate::virtio::device::test_device::{TEST_FEATURE, TestDevice};
use crate::virtio::queue::violation::QueueViolation;
use registers::*;
use status::*;

const W: AccessWidth = AccessWidth::U32;

fn negotiate(t: &mut MmioTransport<TestDevice>) {
    write(t, REG_STATUS, u64::from(STATUS_ACKNOWLEDGE));
    write(t, REG_STATUS, u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER));
    write(t, REG_DRIVER_FEATURES_SEL, 1);
    write(t, REG_DRIVER_FEATURES, 1);
    write(t, REG_DRIVER_FEATURES_SEL, 0);
    write(t, REG_DRIVER_FEATURES, TEST_FEATURE);
    write(
        t,
        REG_STATUS,
        u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK),
    );
    assert!(t.status().features_ok());
}

#[test]
fn features_ok_fails_for_unknown_bits_or_missing_version_1() {
    let mut t = transport();
    write(&mut t, REG_STATUS, 1);
    write(&mut t, REG_STATUS, 3);
    write(&mut t, REG_DRIVER_FEATURES, TEST_FEATURE);
    assert_eq!(
        t.write(REG_STATUS, W, 11, &memory()),
        Err(TransportViolation::FeaturesRejected {
            unsupported: 0,
            missing_version_1: true
        })
    );
    assert_eq!(t.read(REG_STATUS, W), Ok(3), "FEATURES_OK stays clear");
    write(&mut t, REG_DRIVER_FEATURES_SEL, 1);
    write(&mut t, REG_DRIVER_FEATURES, 0x8000_0001);
    assert_eq!(
        t.write(REG_STATUS, W, 11, &memory()),
        Err(TransportViolation::FeaturesRejected {
            unsupported: 1 << 63,
            missing_version_1: false
        })
    );
    write(&mut t, REG_DRIVER_FEATURES, 1);
    write(&mut t, REG_DRIVER_FEATURES_SEL, 5);
    write(&mut t, REG_DRIVER_FEATURES, 0xffff_ffff);
    assert_eq!(write(&mut t, REG_STATUS, 11), TransportEvent::None);
    assert!(t.status().features_ok());
    assert_eq!(
        t.write(REG_DRIVER_FEATURES, W, 0, &memory()),
        Err(TransportViolation::ConfigurationLocked {
            offset: REG_DRIVER_FEATURES
        })
    );
    assert_eq!(t.driver_features(), (1 << 32) | TEST_FEATURE);
}

#[test]
fn status_lifecycle_is_ordered_and_never_clears_bits_without_reset() {
    let mut t = transport();
    let out_of_order = |bit: u8| {
        Err(TransportViolation::Status(StatusViolation::OutOfOrder {
            bit,
        }))
    };
    assert_eq!(t.write(REG_STATUS, W, 2, &memory()), out_of_order(2));
    assert_eq!(t.write(REG_STATUS, W, 8, &memory()), out_of_order(8));
    assert_eq!(t.write(REG_STATUS, W, 4, &memory()), out_of_order(4));
    assert_eq!(
        t.write(REG_STATUS, W, 3, &memory()),
        Err(TransportViolation::Status(StatusViolation::MultipleBits {
            value: 3
        }))
    );
    assert_eq!(
        t.write(REG_STATUS, W, 64, &memory()),
        Err(TransportViolation::Status(
            StatusViolation::DriverSetNeedsReset
        ))
    );
    assert_eq!(
        t.write(REG_STATUS, W, 0x100, &memory()),
        Err(TransportViolation::Status(StatusViolation::UnknownBits {
            value: 0x100
        }))
    );
    write(&mut t, REG_STATUS, 1);
    assert_eq!(write(&mut t, REG_STATUS, 1), TransportEvent::None);
    write(&mut t, REG_STATUS, 3);
    assert_eq!(
        t.write(REG_STATUS, W, 2, &memory()),
        Err(TransportViolation::Status(StatusViolation::ClearedBits {
            current: 3,
            value: 2
        }))
    );
    assert_eq!(write(&mut t, REG_STATUS, 0x03 | 0x80), TransportEvent::None);
    assert!(t.status().is_failed());
    assert!(!t.is_active());
    assert_eq!(write(&mut t, REG_STATUS, 0), TransportEvent::Reset);
    assert_eq!(t.read(REG_STATUS, W), Ok(0));
    assert_eq!(t.device().resets, 1);
}

#[test]
fn queue_selection_and_geometry_are_bounded_and_locked_after_driver_ok() {
    let mem = memory();
    let mut t = transport();
    negotiate(&mut t);
    write(&mut t, REG_QUEUE_SEL, 1);
    assert_eq!(t.read(REG_QUEUE_NUM_MAX, W), Ok(16));
    write(&mut t, REG_QUEUE_SEL, 2);
    assert_eq!(t.read(REG_QUEUE_NUM_MAX, W), Ok(0));
    assert_eq!(t.read(REG_QUEUE_READY, W), Ok(0));
    assert_eq!(
        t.write(REG_QUEUE_NUM, W, 8, &mem),
        Err(TransportViolation::QueueSelOutOfRange { sel: 2 })
    );
    write(&mut t, REG_QUEUE_SEL, 0);
    assert_eq!(
        t.write(REG_QUEUE_NUM, W, 128, &mem),
        Err(TransportViolation::Queue(QueueViolation::Layout(
            LayoutViolation::SizeExceedsMax { size: 128, max: 64 }
        )))
    );
    assert!(matches!(
        t.write(REG_QUEUE_NUM, W, 0x1_0000, &mem),
        Err(TransportViolation::Queue(QueueViolation::Layout(_)))
    ));
    write(&mut t, REG_QUEUE_NUM, 8);
    write(&mut t, REG_QUEUE_DESC_LOW, 0x1000);
    write(&mut t, REG_QUEUE_DESC_HIGH, 0x1);
    write(&mut t, REG_QUEUE_DRIVER_LOW, 0x2000);
    write(&mut t, REG_QUEUE_DEVICE_LOW, 0x3000);
    assert_eq!(
        t.write(REG_QUEUE_READY, W, 1, &mem),
        Err(TransportViolation::Queue(QueueViolation::Layout(
            LayoutViolation::DescOutOfRegion
        )))
    );
    write(&mut t, REG_QUEUE_DESC_HIGH, 0);
    assert_eq!(t.queue(0).expect("queue").state().desc, 0x1000);
    write(&mut t, REG_QUEUE_READY, 1);
    assert_eq!(t.read(REG_QUEUE_READY, W), Ok(1));
    assert_eq!(
        t.write(REG_QUEUE_READY, W, 1, &mem),
        Err(TransportViolation::Queue(QueueViolation::AlreadyActivated))
    );
    assert_eq!(write(&mut t, REG_STATUS, 15), TransportEvent::DriverOk);
    assert_eq!(t.device().activated_with, Some((1 << 32) | TEST_FEATURE));
    for offset in [REG_QUEUE_NUM, REG_QUEUE_READY, REG_QUEUE_DESC_LOW] {
        assert_eq!(
            t.write(offset, W, 1, &mem),
            Err(TransportViolation::ConfigurationLocked { offset })
        );
    }
}

#[test]
fn queue_ready_requires_features_ok_and_notify_requires_driver_ok() {
    let mem = memory();
    let mut t = transport();
    write(&mut t, REG_QUEUE_DESC_LOW, 0x1000);
    write(&mut t, REG_QUEUE_DRIVER_LOW, 0x2000);
    write(&mut t, REG_QUEUE_DEVICE_LOW, 0x3000);
    assert_eq!(
        t.write(REG_QUEUE_READY, W, 1, &mem),
        Err(TransportViolation::ConfigurationLocked {
            offset: REG_QUEUE_READY
        })
    );
    assert_eq!(
        t.write(REG_QUEUE_NOTIFY, W, 0, &mem),
        Err(TransportViolation::NotifyBeforeDriverOk)
    );
    negotiate(&mut t);
    write(&mut t, REG_QUEUE_READY, 1);
    write(&mut t, REG_STATUS, 15);
    assert_eq!(
        write(&mut t, REG_QUEUE_NOTIFY, 0),
        TransportEvent::QueueNotify(0)
    );
    assert_eq!(
        t.write(REG_QUEUE_NOTIFY, W, 1, &mem),
        Err(TransportViolation::NotifyQueueNotReady { index: 1 })
    );
    assert_eq!(
        t.write(REG_QUEUE_NOTIFY, W, 2, &mem),
        Err(TransportViolation::NotifyOutOfRange { index: 2 })
    );
    assert_eq!(
        t.write(REG_QUEUE_NOTIFY, W, 0xffff_ffff, &mem),
        Err(TransportViolation::NotifyOutOfRange { index: 0xffff_ffff })
    );
    t.set_needs_reset();
    assert_eq!(t.read(REG_STATUS, W), Ok(0x0f | 0x40));
    assert_eq!(
        t.write(REG_QUEUE_NOTIFY, W, 0, &mem),
        Err(TransportViolation::NotifyBeforeDriverOk)
    );
    assert!(!t.queue(0).expect("queue").is_ready());
}
