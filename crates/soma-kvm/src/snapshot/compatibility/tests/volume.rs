//! The durable volume's place in the compatibility profile, split out to hold its parent
//! inside the source budget.

use super::*;
use crate::snapshot::manifest::tests::sample_manifest_with_mem;

#[test]
fn a_machine_without_a_volume_carries_the_six_slot_profile_it_always_had() {
    // The compatibility profile is what a machine has, not what the format allows: a Generation
    // that declared no durable volume carries six slot entries and no seventh entry, so its
    // profile is exactly the profile it carried before the volume slot existed.
    let host = matching_host();
    assert_eq!(host.devices.len(), 6);
    assert!(
        host.devices.get(6).is_none(),
        "an undeclared durable volume must add no entry"
    );
    // The six slots the format always had, positional: slot 5 is the optional memory device and
    // is absent on a machine that did not declare it, exactly as before the volume existed.
    assert_eq!(host.devices[5], None);
    assert!(host.devices[..5].iter().all(Option::is_some));
    let manifest = sample_manifest();
    assert_eq!(check(&host, &manifest), Ok(()));
    // Slot 6 exists in the device vocabulary, so an undeclared machine must never be asked for
    // an expectation it cannot have.
    assert_ne!(
        check(&host, &manifest),
        Err(Incompatibility::MissingSection(SectionRole::Device6))
    );
    assert_ne!(
        check(&host, &manifest),
        Err(Incompatibility::UnexpectedSection(SectionRole::Device6))
    );
}

#[test]
fn the_memory_slot_is_required_exactly_when_the_host_declares_it() {
    let with = sample_manifest_with_mem();
    let without = sample_manifest();
    let mut host = matching_host();
    // The common case: neither side has a memory device.
    assert_eq!(check(&host, &without), Ok(()));
    // The machine was captured resizable and this host builds a fixed-size one: refused, because
    // the captured driver negotiated a device this machine would not have.
    assert_eq!(
        check(&host, &with),
        Err(Incompatibility::UnexpectedSection(SectionRole::Device5))
    );
    // The host declares the device and the snapshot has none: refused the other way.
    host.devices[5] = Some(DeviceExpectation {
        kind: DeviceKind::Mem,
        negotiated_features: device_fixtures::features_for(DeviceKind::Mem),
        queue_limits: device_fixtures::queue_limits_for(DeviceKind::Mem),
    });
    assert_eq!(
        check(&host, &without),
        Err(Incompatibility::MissingSection(SectionRole::Device5))
    );
    assert_eq!(check(&host, &with), Ok(()));
}
