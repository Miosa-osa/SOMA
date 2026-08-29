use super::*;
use crate::virtio::guest_memory::VecGuestMemory;
use crate::virtio::queue::chain::{ChainLimits, Descriptor, VIRTQ_DESC_F_WRITE};
use crate::virtio::queue::layout::LayoutViolation;

const DESC: u64 = 0x1000;
const AVAIL: u64 = 0x2000;
const USED: u64 = 0x3000;
const DATA: u64 = 0x4000;
const LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 16,
    max_bytes: 1 << 16,
};

fn memory() -> VecGuestMemory {
    VecGuestMemory::flat(0x8000).expect("memory")
}

fn ready_queue(mem: &VecGuestMemory, size: u16) -> Queue {
    let mut queue = Queue::new(64).expect("max");
    queue.set_size(size).expect("size");
    queue.set_desc_addr(DESC);
    queue.set_avail_addr(AVAIL);
    queue.set_used_addr(USED);
    queue.activate(mem).expect("activate");
    queue
}

fn publish(mem: &VecGuestMemory, size: u16, slot: u16, head: u16, avail_idx: u16) {
    let ring = GuestAddress(AVAIL + 4 + 2 * u64::from(slot % size));
    mem.write_obj_at(ring, head).expect("ring");
    mem.write_obj_at(GuestAddress(AVAIL + 2), avail_idx)
        .expect("idx");
}

fn write_desc(mem: &VecGuestMemory, index: u16, addr: u64, len: u32, flags: u16, next: u16) {
    let descriptor = Descriptor {
        addr,
        len,
        flags,
        next,
    };
    mem.write_bytes(
        GuestAddress(DESC + 16 * u64::from(index)),
        &descriptor.to_bytes(),
    )
    .expect("desc");
}

#[test]
fn le_u64_decodes_short_and_full_slices() {
    assert_eq!(le_u64(&[]), 0);
    assert_eq!(le_u64(&[0x34, 0x12]), 0x1234);
    assert_eq!(le_u64(&[1, 2, 3, 4, 5, 6, 7, 8]), 0x0807_0605_0403_0201);
}

#[test]
fn queue_state_round_trips_and_rejects_bad_records() {
    let mem = memory();
    let mut queue = ready_queue(&mem, 8);
    write_desc(&mem, 0, DATA, 4, VIRTQ_DESC_F_WRITE, 0);
    publish(&mem, 8, 0, 0, 1);
    let chain = queue
        .pop_descriptor_chain(&mem, LIMITS)
        .expect("pop")
        .expect("chain");
    queue.add_used(&mem, &chain, 4).expect("used");
    let state = queue.state();
    let raw = state.to_bytes();
    assert_eq!(raw.len(), QUEUE_STATE_LEN);
    assert_eq!(QueueState::from_bytes(&raw), Ok(state));
    let restored = Queue::restore(&mem, 64, state).expect("restore");
    assert_eq!(restored.state(), state);
    assert!(restored.is_ready());
    assert_eq!(
        QueueState::from_bytes(&raw[..31]),
        Err(QueueStateError::Length { actual: 31 })
    );
    let mut bad = raw;
    bad[2] = 2;
    assert_eq!(
        QueueState::from_bytes(&bad),
        Err(QueueStateError::InvalidFlag { offset: 2 })
    );
}

#[test]
fn restore_fails_closed_on_inconsistent_or_invalid_state() {
    let mem = memory();
    let base = ready_queue(&mem, 8).state();
    let ready_not_activated = QueueState {
        activated: false,
        ..base
    };
    assert_eq!(
        Queue::restore(&mem, 64, ready_not_activated).err(),
        Some(QueueViolation::InconsistentState)
    );
    let cursors_without_activation = QueueState {
        ready: false,
        activated: false,
        next_used: 3,
        ..base
    };
    assert_eq!(
        Queue::restore(&mem, 64, cursors_without_activation).err(),
        Some(QueueViolation::InconsistentState)
    );
    let oversized = QueueState { size: 128, ..base };
    assert!(matches!(
        Queue::restore(&mem, 64, oversized),
        Err(QueueViolation::Layout(
            LayoutViolation::SizeExceedsMax { .. }
        ))
    ));
    let escaped = QueueState {
        used: 0x8000,
        ..base
    };
    assert_eq!(
        Queue::restore(&mem, 64, escaped).err(),
        Some(QueueViolation::Layout(LayoutViolation::UsedOutOfRegion))
    );
    let inactive = QueueState {
        ready: false,
        activated: true,
        next_avail: 7,
        ..base
    };
    let queue = Queue::restore(&mem, 64, inactive).expect("deactivated queue");
    assert!(!queue.is_ready());
    assert_eq!(queue.state().next_avail, 7);
}
