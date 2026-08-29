//! Fail-closed restore and activation-failure behavior.

use super::driver_model_tests::{init_driver, memory};
use super::*;
use crate::virtio::device::test_device::TestDevice;
use crate::virtio::transport::state::RestoreError;
use registers::*;
use status::*;

const W: AccessWidth = AccessWidth::U32;

#[test]
fn restore_fails_closed_on_tampered_state() {
    let mem = memory();
    let mut t = MmioTransport::new(TestDevice::default()).expect("transport");
    init_driver(&mut t, &mem);
    let good = t.state();

    let mut bad_status = good.clone();
    bad_status.status = STATUS_DRIVER_OK;
    assert!(matches!(
        MmioTransport::restore(TestDevice::default(), &bad_status, &mem),
        Err(RestoreError::Status(_))
    ));

    let mut bad_features = good.clone();
    bad_features.driver_features = (1 << 32) | (1 << 40);
    assert!(matches!(
        MmioTransport::restore(TestDevice::default(), &bad_features, &mem),
        Err(RestoreError::Transport(
            TransportViolation::FeaturesRejected { .. }
        ))
    ));

    let mut bad_irq = good.clone();
    bad_irq.interrupt_status = 4;
    assert!(matches!(
        MmioTransport::restore(TestDevice::default(), &bad_irq, &mem),
        Err(RestoreError::InterruptStatus { value: 4 })
    ));

    let mut bad_count = good.clone();
    bad_count.queues.pop();
    assert!(matches!(
        MmioTransport::restore(TestDevice::default(), &bad_count, &mem),
        Err(RestoreError::QueueCount { .. })
    ));

    let mut bad_queue = good.clone();
    bad_queue.queues[0].used = 0x7ffc;
    assert!(matches!(
        MmioTransport::restore(TestDevice::default(), &bad_queue, &mem),
        Err(RestoreError::Queue { index: 0, .. })
    ));

    let mut ready_before_features = good.clone();
    ready_before_features.status = STATUS_ACKNOWLEDGE | STATUS_DRIVER;
    assert!(matches!(
        MmioTransport::restore(TestDevice::default(), &ready_before_features, &mem),
        Err(RestoreError::Queue { index: 0, .. })
    ));

    let rejecting = TestDevice {
        reject_activation: true,
        ..TestDevice::default()
    };
    assert!(matches!(
        MmioTransport::restore(rejecting, &good, &mem),
        Err(RestoreError::Transport(TransportViolation::Activate(_)))
    ));
}

#[test]
fn activation_failure_marks_needs_reset_and_reset_recovers() {
    let mem = memory();
    let device = TestDevice {
        reject_activation: true,
        ..TestDevice::default()
    };
    let mut t = MmioTransport::new(device).expect("transport");
    let w = |t: &mut MmioTransport<TestDevice>, offset: u64, value: u64| {
        t.write(offset, W, value, &mem)
    };
    w(&mut t, REG_STATUS, 1).expect("ack");
    w(&mut t, REG_STATUS, 3).expect("driver");
    w(&mut t, REG_DRIVER_FEATURES_SEL, 1).expect("sel");
    w(&mut t, REG_DRIVER_FEATURES, 1).expect("features");
    w(&mut t, REG_STATUS, 11).expect("features ok");
    assert!(matches!(
        w(&mut t, REG_STATUS, 15),
        Err(TransportViolation::Activate(_))
    ));
    assert_eq!(t.read(REG_STATUS, W), Ok(0x0b | 0x40));
    assert!(!t.is_active());
    assert_eq!(w(&mut t, REG_STATUS, 0), Ok(TransportEvent::Reset));
    assert_eq!(t.read(REG_STATUS, W), Ok(0));
    assert_eq!(t.driver_features(), 0);
    assert_eq!(t.device().resets, 1);
}
