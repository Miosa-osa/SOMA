//! Device sections a snapshot carries, checked against the devices this host expects.

use super::{DeviceState, HostProfile, Incompatibility, Manifest, StateView, device_role};

/// Decodes one device section and compares queue limits and negotiated features with the
/// host expectation for that slot.
///
/// # Errors
///
/// Returns the first device [`Incompatibility`] for `slot`.
pub fn check_device(
    host: &HostProfile,
    manifest: &Manifest,
    slot: u8,
) -> Result<(), Incompatibility> {
    check_state_device(host, manifest.state(), slot)
}

pub(super) fn check_state_device(
    host: &HostProfile,
    manifest: StateView<'_>,
    slot: u8,
) -> Result<(), Incompatibility> {
    let expectation = host
        .devices
        .get(usize::from(slot))
        .and_then(|expectation| expectation.as_ref())
        .filter(|expectation| expectation.kind.slot() == slot)
        .ok_or(Incompatibility::NoExpectationForSlot(slot))?;
    let role = device_role(slot).ok_or(Incompatibility::NoExpectationForSlot(slot))?;
    let section = manifest
        .section(role)
        .ok_or(Incompatibility::MissingSection(role))?;
    let state = DeviceState::decode_for_slot(slot, section.payload())
        .map_err(|error| Incompatibility::MalformedDevice { slot, error })?;
    for (index, queue) in state.queues().iter().enumerate() {
        let expected = expectation.queue_limits[index];
        if queue.max_size != expected {
            return Err(Incompatibility::QueueLimit {
                slot,
                queue: u8::try_from(index).unwrap_or(u8::MAX),
                expected,
                actual: queue.max_size,
            });
        }
    }
    exact(
        expectation.negotiated_features,
        state.negotiated_features(),
        |expected, actual| Incompatibility::FeatureNegotiation {
            slot,
            expected,
            actual,
        },
    )
}

pub(super) fn exact<T: PartialEq + Copy>(
    expected: T,
    actual: T,
    reason: impl FnOnce(T, T) -> Incompatibility,
) -> Result<(), Incompatibility> {
    if expected == actual {
        Ok(())
    } else {
        Err(reason(expected, actual))
    }
}
