//! Attaching one assigned bundle's frame path to a device built without one.

use super::backend::LoopbackBackend;
use super::{NET_TX_QUEUE, NetDevice};

const BUNDLE_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x7f];
const BUILT_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];

fn unattached() -> NetDevice {
    NetDevice::new(Box::new(LoopbackBackend::default()), BUILT_MAC)
}

/// The address identity arrives with the frame path, because a device serving one Instance's
/// frames under another's address is the confusion this ordering exists to prevent.
#[test]
fn attaching_a_bundle_takes_its_address_identity() {
    let mut device = unattached();
    assert_eq!(device.mac(), BUILT_MAC);

    device.attach(Box::new(LoopbackBackend::default()), BUNDLE_MAC);

    assert_eq!(device.mac(), BUNDLE_MAC);
}

/// Attaching a frame path is not permission to use it: the link is a separate admitted step.
#[test]
fn attaching_a_bundle_does_not_raise_the_link() {
    let mut device = unattached();
    assert!(!device.link_up(), "a device is built with its link down");

    device.attach(Box::new(LoopbackBackend::default()), BUNDLE_MAC);

    assert!(
        !device.link_up(),
        "attaching a frame path raised the link on its own"
    );
}

/// A raised link stays raised, so attaching is not a way to silently interrupt a live Instance.
#[test]
fn attaching_leaves_a_raised_link_alone() {
    let mut device = unattached();
    device.set_link(true);

    device.attach(Box::new(LoopbackBackend::default()), BUNDLE_MAC);

    assert!(device.link_up());
    assert_eq!(
        NET_TX_QUEUE, 1,
        "the transmit queue index is part of the contract"
    );
}
