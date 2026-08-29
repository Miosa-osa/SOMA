use super::*;
use crate::virtio::guest_memory::VecGuestMemory;

const TABLE: GuestAddress = GuestAddress(0x1000);
const DATA: u64 = 0x4000;
const SIZE: u16 = 8;
const LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 64,
    max_bytes: 1 << 20,
};

fn memory() -> VecGuestMemory {
    VecGuestMemory::flat(0x10000).expect("memory")
}

fn write_table(mem: &VecGuestMemory, descriptors: &[Descriptor]) {
    for (index, descriptor) in descriptors.iter().enumerate() {
        let addr = TABLE
            .checked_add(index as u64 * DESCRIPTOR_SIZE)
            .expect("in range");
        mem.write_bytes(addr, &descriptor.to_bytes())
            .expect("write");
    }
}

fn desc(addr: u64, len: u32, flags: u16, next: u16) -> Descriptor {
    Descriptor {
        addr,
        len,
        flags,
        next,
    }
}

fn walk(mem: &VecGuestMemory, head: u16) -> Result<DescriptorChain, ChainViolation> {
    walk_chain(mem, TABLE, SIZE, head, LIMITS)
}

#[test]
fn descriptor_bytes_round_trip() {
    let descriptor = desc(0x1122_3344_5566_7788, 0xaabb_ccdd, 0x0102, 0x0304);
    assert_eq!(Descriptor::from_bytes(descriptor.to_bytes()), descriptor);
    let raw = descriptor.to_bytes();
    assert_eq!(&raw[0..2], &[0x88, 0x77]);
    assert_eq!(&raw[12..14], &[0x02, 0x01]);
}

#[test]
fn walks_readable_then_writable_chain() {
    let mem = memory();
    write_table(
        &mem,
        &[
            desc(DATA, 16, VIRTQ_DESC_F_NEXT, 3),
            desc(0, 0, 0, 0),
            desc(0, 0, 0, 0),
            desc(DATA + 16, 32, VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE, 5),
            desc(0, 0, 0, 0),
            desc(DATA + 64, 8, VIRTQ_DESC_F_WRITE, 0),
        ],
    );
    let chain = walk(&mem, 0).expect("valid chain");
    assert_eq!(chain.head(), 0);
    assert_eq!(chain.segments().len(), 3);
    assert_eq!(chain.readable_len(), 16);
    assert_eq!(chain.writable_len(), 40);
    assert_eq!(chain.readable().count(), 1);
    assert_eq!(chain.writable().count(), 2);
    assert_eq!(
        chain.segments()[1],
        ChainSegment {
            addr: GuestAddress(DATA + 16),
            len: 32,
            writable: true
        }
    );
}

#[test]
fn rejects_head_and_next_outside_queue_size() {
    let mem = memory();
    write_table(&mem, &[desc(DATA, 4, VIRTQ_DESC_F_NEXT, SIZE)]);
    assert_eq!(
        walk(&mem, SIZE),
        Err(ChainViolation::IndexOutOfRange {
            index: SIZE,
            size: SIZE
        })
    );
    assert_eq!(
        walk(&mem, 0),
        Err(ChainViolation::IndexOutOfRange {
            index: SIZE,
            size: SIZE
        })
    );
}

#[test]
fn rejects_loops_self_references_and_over_long_chains() {
    let mem = memory();
    write_table(
        &mem,
        &[
            desc(DATA, 4, VIRTQ_DESC_F_NEXT, 1),
            desc(DATA, 4, VIRTQ_DESC_F_NEXT, 0),
            desc(DATA, 4, VIRTQ_DESC_F_NEXT, 2),
        ],
    );
    assert_eq!(
        walk(&mem, 0),
        Err(ChainViolation::RepeatedIndex { index: 0 })
    );
    assert_eq!(
        walk(&mem, 2),
        Err(ChainViolation::RepeatedIndex { index: 2 })
    );
    let ring: Vec<Descriptor> = (0..SIZE)
        .map(|i| desc(DATA, 4, VIRTQ_DESC_F_NEXT, (i + 1) % SIZE))
        .collect();
    write_table(&mem, &ring);
    assert_eq!(
        walk(&mem, 0),
        Err(ChainViolation::RepeatedIndex { index: 0 })
    );
    let tight = ChainLimits {
        max_descriptors: 2,
        max_bytes: 1 << 20,
    };
    assert_eq!(
        walk_chain(&mem, TABLE, SIZE, 0, tight),
        Err(ChainViolation::TooLong { limit: 2 })
    );
}

#[test]
fn rejects_indirect_unknown_flags_and_zero_length() {
    let mem = memory();
    write_table(
        &mem,
        &[
            desc(DATA, 4, VIRTQ_DESC_F_INDIRECT, 0),
            desc(DATA, 4, 0x8, 0),
            desc(DATA, 0, 0, 0),
        ],
    );
    assert_eq!(walk(&mem, 0), Err(ChainViolation::Indirect { index: 0 }));
    assert_eq!(
        walk(&mem, 1),
        Err(ChainViolation::UnknownFlags { index: 1 })
    );
    assert_eq!(walk(&mem, 2), Err(ChainViolation::ZeroLength { index: 2 }));
}

#[test]
fn rejects_overflow_out_of_region_and_direction_order() {
    let mem = memory();
    write_table(
        &mem,
        &[
            desc(u64::MAX - 1, 4, 0, 0),
            desc(0x10000 - 2, 4, 0, 0),
            desc(DATA, 4, VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT, 3),
            desc(DATA, 4, 0, 0),
        ],
    );
    assert_eq!(
        walk(&mem, 0),
        Err(ChainViolation::AddressOverflow { index: 0 })
    );
    assert_eq!(walk(&mem, 1), Err(ChainViolation::OutOfRegion { index: 1 }));
    assert_eq!(
        walk(&mem, 2),
        Err(ChainViolation::ReadableAfterWritable { index: 3 })
    );
}

#[test]
fn rejects_aggregate_bytes_above_limit_without_wrapping() {
    let mem = memory();
    write_table(
        &mem,
        &[
            desc(DATA, 0x100, VIRTQ_DESC_F_NEXT, 1),
            desc(DATA, 0x100, VIRTQ_DESC_F_WRITE, 0),
        ],
    );
    let limit = ChainLimits {
        max_descriptors: 64,
        max_bytes: 0x1ff,
    };
    assert_eq!(
        walk_chain(&mem, TABLE, SIZE, 0, limit),
        Err(ChainViolation::BytesExceeded { limit: 0x1ff })
    );
    let exact = ChainLimits {
        max_descriptors: 64,
        max_bytes: 0x200,
    };
    assert!(walk_chain(&mem, TABLE, SIZE, 0, exact).is_ok());
}

#[test]
fn rejects_invalid_queue_size_and_unreadable_table() {
    let mem = memory();
    for size in [0, 3, MAX_QUEUE_SIZE + 1, u16::MAX] {
        assert_eq!(
            walk_chain(&mem, TABLE, size, 0, LIMITS),
            Err(ChainViolation::InvalidQueueSize { size })
        );
    }
    assert_eq!(
        walk_chain(&mem, GuestAddress(0x2_0000), SIZE, 0, LIMITS),
        Err(ChainViolation::DescriptorUnreadable { index: 0 })
    );
    assert_eq!(
        walk_chain(&mem, GuestAddress(u64::MAX - 8), SIZE, 1, LIMITS),
        Err(ChainViolation::DescriptorUnreadable { index: 1 })
    );
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Property: on random hostile tables the walk always terminates with a chain
/// whose every segment lies in registered memory, obeys direction order, and
/// respects the limits, or a typed violation; it never panics.
#[test]
fn hostile_random_tables_never_panic_and_accepted_chains_are_sound() {
    let mem = memory();
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    let mut accepted = 0u32;
    for round in 0..2_000u32 {
        let size = 1u16 << (rng.next() % 6);
        let table: Vec<Descriptor> = (0..size)
            .map(|_| {
                let addr = match rng.next() % 8 {
                    0 => rng.next(),
                    1 => u64::MAX - rng.next() % 64,
                    2 => 0x10000 - rng.next() % 16,
                    _ => DATA + rng.next() % 0x8000,
                };
                let len = match rng.next() % 8 {
                    0 => 0,
                    1 => u32::try_from(rng.next() % u64::from(u32::MAX)).expect("fits"),
                    _ => u32::try_from(1 + rng.next() % 0x200).expect("small"),
                };
                let flags = if rng.next().is_multiple_of(4) {
                    u16::try_from(rng.next() % 16).expect("fits")
                } else {
                    u16::try_from(rng.next() % 4).expect("fits")
                };
                let next = u16::try_from(rng.next() % (u64::from(size) + 2)).expect("fits");
                desc(addr, len, flags, next)
            })
            .collect();
        write_table(&mem, &table);
        let limits = ChainLimits {
            max_descriptors: u16::try_from(1 + rng.next() % 40).expect("fits"),
            max_bytes: 0x400 + rng.next() % 0x10000,
        };
        let head = u16::try_from(rng.next() % (u64::from(size) + 1)).expect("fits");
        let Ok(chain) = walk_chain(&mem, TABLE, size, head, limits) else {
            continue;
        };
        accepted += 1;
        assert!(!chain.segments().is_empty(), "round {round}");
        assert!(chain.segments().len() <= usize::from(limits.max_descriptors.min(size)));
        assert!(chain.readable_len() + chain.writable_len() <= limits.max_bytes);
        let mut seen_writable = false;
        for segment in chain.segments() {
            assert!(segment.len > 0);
            mem.check_range(segment.addr, u64::from(segment.len))
                .expect("accepted segment is contained");
            assert!(!seen_writable || segment.writable, "direction order");
            seen_writable |= segment.writable;
        }
    }
    assert!(
        accepted > 50,
        "property test accepted only {accepted} chains"
    );
}
