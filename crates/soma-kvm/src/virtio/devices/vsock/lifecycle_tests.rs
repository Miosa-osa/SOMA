//! Reset, restore, configuration, and constructor behavior.

use super::guest_driver::{CID, GuestVsock, connect, hdr};
use super::packet::*;
use super::*;

#[test]
fn reset_and_restore_clear_connections_and_restore_queues_transport_reset() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    guest.device().endpoint().expect("endpoint").write(b"x");
    let raw = guest.device().snapshot_state();
    assert_eq!(raw.len(), state::VSOCK_STATE_LEN);
    let mut idle = VsockDevice::new(CID).expect("device");
    assert_eq!(
        idle.snapshot_state(),
        raw,
        "connection state never enters the record"
    );
    assert_eq!(idle.restore_state(&raw), Ok(()));
    assert!(idle.endpoint().is_none());
    assert_eq!(idle.pending_events(), 1);
    assert_eq!(idle.generation(), 1);
    assert!(!idle.is_quiescent());
    assert_eq!(
        VsockDevice::new(CID)
            .expect("device")
            .restore_state(&raw[..20]),
        Err(DeviceStateError::Malformed)
    );
    let mut wrong_id = raw.clone();
    wrong_id[1] = 4;
    assert_eq!(
        idle.restore_state(&wrong_id),
        Err(DeviceStateError::Incompatible)
    );
    let mut wrong_features = raw.clone();
    wrong_features[5] = 1;
    assert_eq!(
        idle.restore_state(&wrong_features),
        Err(DeviceStateError::Incompatible)
    );
    let mut reserved_cid = raw.clone();
    reserved_cid[13..21].copy_from_slice(&2u64.to_le_bytes());
    assert_eq!(
        idle.restore_state(&reserved_cid),
        Err(DeviceStateError::Incompatible)
    );

    guest
        .t
        .device_mut()
        .restore_state(&raw)
        .expect("restore in place");
    guest.post_event();
    guest.post_event();
    assert_eq!(guest.events(), vec![VSOCK_EVENT_TRANSPORT_RESET]);
    assert!(guest.device().is_quiescent());
    guest.post_rx(4096);
    guest.send(hdr(VSOCK_OP_RW, 1, 0, 0), b"z");
    assert_eq!(
        guest.recv()[0].0.op,
        VSOCK_OP_RST,
        "stale connection is reset"
    );
    connect(&mut guest);
    assert_eq!(guest.device().endpoint().expect("endpoint").generation(), 4);
    guest.rig.init(&mut guest.t, VSOCK_FEATURES);
    assert!(guest.t.device().is_activated());
    assert!(
        guest.t.device_mut().endpoint().is_none(),
        "driver reset clears the connection"
    );
}

#[test]
fn config_exposes_the_cid_and_constructor_rejects_reserved_cids() {
    use crate::virtio::transport::registers::AccessWidth;
    let mut guest = GuestVsock::boot();
    assert_eq!(guest.t.read(0x100, AccessWidth::U64), Ok(CID));
    assert_eq!(guest.t.read(0x100, AccessWidth::U32), Ok(CID));
    assert_eq!(guest.t.read(0x104, AccessWidth::U32), Ok(0));
    assert!(
        guest
            .t
            .write(0x100, AccessWidth::U32, 9, &guest.rig.mem)
            .is_err()
    );
    for cid in [0, 1, 2, CID_ANY, u64::MAX] {
        assert_eq!(
            VsockDevice::new(cid).err(),
            Some(VsockConfigError::InvalidCid { cid })
        );
    }
    assert!(guest.device().set_guest_cid(2).is_err());
    assert!(guest.device().set_guest_cid(77).is_ok());
    assert_eq!(guest.t.read(0x100, AccessWidth::U64), Ok(77));
}
