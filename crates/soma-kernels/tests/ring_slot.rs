//! `RingSize::slot` equals the remainder for every index and every size a ring may have.

use soma_kernels::ring::{RingSize, ring_slot_reference};

#[test]
fn every_index_for_every_power_of_two_size() {
    for shift in 0..16 {
        let size = 1_u16 << shift;
        let ring = RingSize::new(size).expect("a power of two is a ring size");
        assert_eq!(ring.get(), size);
        for index in 0..=u16::MAX {
            assert_eq!(
                ring.slot(index),
                ring_slot_reference(index, size),
                "index {index} size {size}"
            );
        }
    }
}

#[test]
fn every_other_size_is_refused() {
    // Every 16-bit value: zero and each non-power-of-two have no ring size, so no slot is ever
    // computed for them.
    for size in 0..=u16::MAX {
        assert_eq!(
            RingSize::new(size).is_some(),
            size.is_power_of_two(),
            "size {size}"
        );
    }
}

#[test]
fn wraparound_of_the_free_running_index() {
    for size in [1_u16, 2, 4, 128, 256, 1_024, 32_768] {
        let ring = RingSize::new(size).expect("power of two");
        let mut index = u16::MAX - 3;
        for _ in 0..8 {
            assert_eq!(ring.slot(index), index % size);
            index = index.wrapping_add(1);
        }
    }
}
