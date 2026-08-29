use super::*;
use crate::virtio::guest_memory::{GuestMemoryError, VecGuestMemory};
use chain::{Descriptor, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};
use violation::QueueViolationKind;

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
fn new_rejects_invalid_maximum_and_set_size_enforces_bounds() {
    assert!(matches!(Queue::new(0), Err(LayoutViolation::SizeZero)));
    assert!(matches!(
        Queue::new(48),
        Err(LayoutViolation::SizeNotPowerOfTwo { size: 48 })
    ));
    let mut queue = Queue::new(64).expect("max");
    assert_eq!(queue.size(), 64);
    assert_eq!(
        queue.set_size(128),
        Err(QueueViolation::Layout(LayoutViolation::SizeExceedsMax {
            size: 128,
            max: 64
        }))
    );
    assert_eq!(
        queue.set_size(0),
        Err(QueueViolation::Layout(LayoutViolation::SizeZero))
    );
    queue.set_size(16).expect("valid");
    assert_eq!(queue.size(), 16);
    assert_eq!(queue.violations().count(QueueViolationKind::Layout), 2);
}

#[test]
fn activation_validates_alignment_containment_overlap_and_single_use() {
    let mem = memory();
    let mut queue = Queue::new(64).expect("max");
    queue.set_desc_addr(DESC + 8);
    queue.set_avail_addr(AVAIL);
    queue.set_used_addr(USED);
    assert_eq!(
        queue.activate(&mem),
        Err(QueueViolation::Layout(LayoutViolation::DescMisaligned))
    );
    queue.set_desc_addr(DESC);
    queue.set_avail_addr(AVAIL + 1);
    assert_eq!(
        queue.activate(&mem),
        Err(QueueViolation::Layout(LayoutViolation::AvailMisaligned))
    );
    queue.set_avail_addr(AVAIL);
    queue.set_used_addr(USED + 2);
    assert_eq!(
        queue.activate(&mem),
        Err(QueueViolation::Layout(LayoutViolation::UsedMisaligned))
    );
    queue.set_used_addr(0x8000 - 4);
    assert_eq!(
        queue.activate(&mem),
        Err(QueueViolation::Layout(LayoutViolation::UsedOutOfRegion))
    );
    queue.set_used_addr(DESC + 16);
    assert_eq!(
        queue.activate(&mem),
        Err(QueueViolation::Layout(LayoutViolation::RingsOverlap))
    );
    queue.set_used_addr(USED);
    assert!(!queue.is_ready());
    queue.activate(&mem).expect("valid");
    assert!(queue.is_ready());
    assert_eq!(queue.activate(&mem), Err(QueueViolation::AlreadyActivated));
    queue.deactivate();
    assert!(!queue.is_ready());
    assert_eq!(queue.activate(&mem), Err(QueueViolation::AlreadyActivated));
    queue.reset();
    queue.set_desc_addr(DESC);
    queue.set_avail_addr(AVAIL);
    queue.set_used_addr(USED);
    queue.activate(&mem).expect("reset permits reactivation");
    assert_eq!(
        queue
            .violations()
            .count(QueueViolationKind::AlreadyActivated),
        2
    );
}

#[test]
fn pop_requires_ready_and_returns_none_when_idle() {
    let mem = memory();
    let mut queue = Queue::new(8).expect("max");
    assert_eq!(
        queue.pop_descriptor_chain(&mem, LIMITS),
        Err(QueueViolation::NotReady)
    );
    let mut queue = ready_queue(&mem, 8);
    assert_eq!(queue.pending(&mem), Ok(0));
    assert_eq!(queue.pop_descriptor_chain(&mem, LIMITS), Ok(None));
}

#[test]
fn pop_walks_chain_and_add_used_publishes_element_and_index() {
    let mem = memory();
    let mut queue = ready_queue(&mem, 8);
    write_desc(&mem, 3, DATA, 8, VIRTQ_DESC_F_NEXT, 5);
    write_desc(&mem, 5, DATA + 8, 24, VIRTQ_DESC_F_WRITE, 0);
    publish(&mem, 8, 0, 3, 1);
    assert_eq!(queue.pending(&mem), Ok(1));
    let chain = queue
        .pop_descriptor_chain(&mem, LIMITS)
        .expect("pop")
        .expect("chain");
    assert_eq!(chain.head(), 3);
    assert_eq!(chain.writable_len(), 24);
    assert_eq!(queue.pending(&mem), Ok(0));
    assert_eq!(
        queue.add_used(&mem, &chain, 25),
        Err(QueueViolation::UsedLengthExceedsCapacity {
            len: 25,
            capacity: 24
        })
    );
    queue.add_used(&mem, &chain, 20).expect("used");
    assert_eq!(mem.read_obj_at::<u32>(GuestAddress(USED + 4)), Ok(3));
    assert_eq!(mem.read_obj_at::<u32>(GuestAddress(USED + 8)), Ok(20));
    assert_eq!(mem.read_obj_at::<u16>(GuestAddress(USED + 2)), Ok(1));
    assert_eq!(queue.state().next_avail, 1);
    assert_eq!(queue.state().next_used, 1);
}

#[test]
fn cursors_wrap_across_ring_boundary() {
    let mem = memory();
    let mut queue = ready_queue(&mem, 4);
    write_desc(&mem, 0, DATA, 4, VIRTQ_DESC_F_WRITE, 0);
    for round in 0..10u16 {
        publish(&mem, 4, round, 0, round + 1);
        let chain = queue
            .pop_descriptor_chain(&mem, LIMITS)
            .expect("pop")
            .expect("chain");
        queue.add_used(&mem, &chain, 4).expect("used");
    }
    assert_eq!(queue.state().next_avail, 10);
    assert_eq!(mem.read_obj_at::<u16>(GuestAddress(USED + 2)), Ok(10));
    assert_eq!(mem.read_obj_at::<u32>(GuestAddress(USED + 4 + 8)), Ok(0));
}

#[test]
fn avail_index_advanced_beyond_size_is_a_violation() {
    let mem = memory();
    let mut queue = ready_queue(&mem, 8);
    publish(&mem, 8, 0, 0, 9);
    assert_eq!(
        queue.pending(&mem),
        Err(QueueViolation::AvailIndexOverrun {
            pending: 9,
            size: 8
        })
    );
    assert_eq!(
        queue
            .violations()
            .count(QueueViolationKind::AvailIndexOverrun),
        1
    );
}

#[test]
fn bad_chain_is_consumed_and_reported_with_its_head() {
    let mem = memory();
    let mut queue = ready_queue(&mem, 8);
    write_desc(&mem, 2, DATA, 4, VIRTQ_DESC_F_NEXT, 9);
    publish(&mem, 8, 0, 2, 1);
    assert!(matches!(
        queue.pop_descriptor_chain(&mem, LIMITS),
        Err(QueueViolation::Chain { head: 2, .. })
    ));
    assert_eq!(queue.pending(&mem), Ok(0), "hostile head does not spin");
    assert_eq!(queue.violations().count(QueueViolationKind::Chain), 1);
}

#[test]
fn needs_notification_honors_no_interrupt_flag_only() {
    let mem = memory();
    let mut queue = ready_queue(&mem, 8);
    assert_eq!(queue.needs_notification(&mem), Ok(true));
    mem.write_obj_at(GuestAddress(AVAIL), VIRTQ_AVAIL_F_NO_INTERRUPT)
        .expect("flags");
    assert_eq!(queue.needs_notification(&mem), Ok(false));
    mem.write_obj_at(GuestAddress(AVAIL), 0x8000u16)
        .expect("flags");
    assert_eq!(queue.needs_notification(&mem), Ok(true));
}

#[test]
fn ring_access_outside_memory_is_a_memory_violation() {
    let small = VecGuestMemory::flat(0x8000).expect("memory");
    let mut queue = ready_queue(&small, 8);
    let other = VecGuestMemory::flat(0x1000).expect("memory");
    assert_eq!(
        queue.pending(&other),
        Err(QueueViolation::Memory(GuestMemoryError::OutOfRegion {
            addr: GuestAddress(AVAIL + 2),
            len: 2
        }))
    );
    assert_eq!(queue.violations().count(QueueViolationKind::Memory), 1);
    assert_eq!(queue.violations().total(), 1);
}
