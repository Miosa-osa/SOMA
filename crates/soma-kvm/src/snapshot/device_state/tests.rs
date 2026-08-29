use super::{
    BlockState, DeviceKind, DeviceSpecific, DeviceState, DeviceStateError, QueueState,
    TransportState,
};
use crate::snapshot::Digest;

pub(crate) const VIRTIO_F_VERSION_1: u64 = 1 << 32;
pub(crate) const VIRTIO_BLK_F_RO: u64 = 1 << 5;
pub(crate) const VIRTIO_BLK_F_BLK_SIZE: u64 = 1 << 6;
pub(crate) const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
pub(crate) const VIRTIO_NET_F_MAC: u64 = 1 << 5;

pub(crate) fn queue(max_size: u16) -> QueueState {
    QueueState {
        max_size,
        size: max_size,
        ready: true,
        descriptor_address: 0x1000,
        available_address: 0x2000,
        used_address: 0x3000,
        next_available: 3,
        next_used: 3,
    }
}

fn transport() -> TransportState {
    TransportState {
        device_status: 0x0f,
        interrupt_status: 0,
        config_generation: 1,
        queue_select: 0,
    }
}

pub(crate) fn features_for(kind: DeviceKind) -> u64 {
    match kind {
        DeviceKind::RootBlock => VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_RO | VIRTIO_BLK_F_BLK_SIZE,
        DeviceKind::OverlayBlock => VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_BLK_SIZE | VIRTIO_BLK_F_FLUSH,
        DeviceKind::Net => VIRTIO_F_VERSION_1 | VIRTIO_NET_F_MAC,
        DeviceKind::Vsock | DeviceKind::Rng => VIRTIO_F_VERSION_1,
    }
}

pub(crate) fn queue_limits_for(kind: DeviceKind) -> [u16; 3] {
    match kind {
        DeviceKind::RootBlock | DeviceKind::OverlayBlock => [256, 0, 0],
        DeviceKind::Net => [256, 256, 0],
        DeviceKind::Vsock => [256, 256, 64],
        DeviceKind::Rng => [64, 0, 0],
    }
}

pub(crate) fn sample(kind: DeviceKind) -> DeviceState {
    let queues = queue_limits_for(kind)[..kind.queue_count()]
        .iter()
        .map(|limit| queue(*limit))
        .collect();
    let specific = match kind {
        DeviceKind::RootBlock | DeviceKind::OverlayBlock => DeviceSpecific::Block(BlockState {
            capacity_sectors: 1 << 20,
            block_size: 4096,
            image_digest: Digest::of(&[kind.slot()]),
        }),
        DeviceKind::Net => DeviceSpecific::Net {
            mac: [0x02, 0, 0, 0, 0, 1],
            link_up: false,
        },
        DeviceKind::Vsock => DeviceSpecific::Vsock { cid_placeholder: 3 },
        DeviceKind::Rng => DeviceSpecific::Rng,
    };
    DeviceState::new(kind, transport(), features_for(kind), queues, specific).unwrap()
}

#[test]
fn every_device_kind_round_trips_through_its_slot() {
    for kind in DeviceKind::ALL {
        let state = sample(kind);
        let bytes = state.encode();
        assert_eq!(DeviceState::decode_for_slot(kind.slot(), &bytes), Ok(state));
        let wrong_slot = (kind.slot() + 1) % 5;
        assert_eq!(
            DeviceState::decode_for_slot(wrong_slot, &bytes),
            Err(DeviceStateError::KindSlotMismatch {
                slot: wrong_slot,
                kind
            })
        );
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(matches!(
            DeviceState::decode_for_slot(kind.slot(), &extended),
            Err(DeviceStateError::Wire(_))
        ));
        for length in 0..bytes.len() {
            assert!(DeviceState::decode_for_slot(kind.slot(), &bytes[..length]).is_err());
        }
    }
}

#[test]
fn rejects_queue_count_specific_mismatch_and_bad_fields() {
    let block = DeviceSpecific::Block(BlockState {
        capacity_sectors: 8,
        block_size: 512,
        image_digest: Digest::of(b"x"),
    });
    assert_eq!(
        DeviceState::new(DeviceKind::Net, transport(), 0, vec![queue(256)], block),
        Err(DeviceStateError::QueueCount {
            kind: DeviceKind::Net,
            count: 1
        })
    );
    assert_eq!(
        DeviceState::new(DeviceKind::Rng, transport(), 0, vec![queue(64)], block),
        Err(DeviceStateError::SpecificMismatch(DeviceKind::Rng))
    );
    let bad_block = DeviceSpecific::Block(BlockState {
        capacity_sectors: u64::MAX,
        block_size: 512,
        image_digest: Digest::of(b"x"),
    });
    assert_eq!(
        DeviceState::new(
            DeviceKind::RootBlock,
            transport(),
            0,
            vec![queue(256)],
            bad_block
        ),
        Err(DeviceStateError::InvalidField {
            field: "capacity_sectors",
            value: u64::MAX
        })
    );
    let mut irq = transport();
    irq.interrupt_status = 4;
    assert_eq!(
        DeviceState::new(
            DeviceKind::Rng,
            irq,
            0,
            vec![queue(64)],
            DeviceSpecific::Rng
        ),
        Err(DeviceStateError::InvalidField {
            field: "interrupt_status",
            value: 4
        })
    );
    let mut unaligned = queue(64);
    unaligned.size = 48;
    assert_eq!(
        DeviceState::new(
            DeviceKind::Rng,
            transport(),
            0,
            vec![unaligned],
            DeviceSpecific::Rng
        ),
        Err(DeviceStateError::InvalidQueue {
            index: 0,
            field: "size"
        })
    );
    let mut not_ready = queue(64);
    not_ready.size = 0;
    assert_eq!(
        not_ready.validate(0),
        Err(DeviceStateError::InvalidQueue {
            index: 0,
            field: "ready"
        })
    );
    assert_eq!(
        DeviceState::decode_for_slot(0, &[9]),
        Err(DeviceStateError::UnknownKind(9))
    );
}
