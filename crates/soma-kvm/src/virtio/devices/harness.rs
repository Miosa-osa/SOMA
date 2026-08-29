//! A model guest driver for device tests: initializes a transport through
//! the real register interface and publishes descriptor chains into
//! in-memory guest RAM.

use crate::virtio::device::VirtioDevice;
use crate::virtio::guest_memory::{GuestAddress, GuestMemory, VecGuestMemory};
use crate::virtio::queue::chain::{Descriptor, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};
use crate::virtio::transport::registers::*;
use crate::virtio::transport::status::*;
use crate::virtio::transport::{MmioTransport, TransportEvent};

const W: AccessWidth = AccessWidth::U32;
/// Total guest RAM; large enough for a 1 MiB block request plus rings.
pub(crate) const MEM_LEN: usize = 4 << 20;
const QUEUE_BASE: u64 = 0x1000;
const QUEUE_STRIDE: u64 = 0x4000;
const DATA_BASE: u64 = 0x40000;

/// One segment of a chain to publish.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg {
    pub addr: u64,
    pub len: u32,
    pub writable: bool,
}

impl Seg {
    pub(crate) const fn readable(addr: u64, len: u32) -> Self {
        Self {
            addr,
            len,
            writable: false,
        }
    }

    pub(crate) const fn writable(addr: u64, len: u32) -> Self {
        Self {
            addr,
            len,
            writable: true,
        }
    }
}

pub(crate) struct GuestRig {
    pub mem: VecGuestMemory,
    sizes: Vec<u16>,
    next_desc: Vec<u16>,
    avail_idx: Vec<u16>,
    next_data: u64,
}

impl GuestRig {
    /// A rig whose queues use the given driver-selected sizes (each at most 128).
    pub(crate) fn new(sizes: &[u16]) -> Self {
        assert!(sizes.iter().all(|size| *size <= 128));
        Self {
            mem: VecGuestMemory::flat(MEM_LEN).expect("memory"),
            sizes: sizes.to_vec(),
            next_desc: vec![0; sizes.len()],
            avail_idx: vec![0; sizes.len()],
            next_data: DATA_BASE,
        }
    }

    fn desc(queue: u16) -> u64 {
        QUEUE_BASE + QUEUE_STRIDE * u64::from(queue)
    }

    fn avail(queue: u16) -> u64 {
        Self::desc(queue) + 0x1000
    }

    fn used(queue: u16) -> u64 {
        Self::desc(queue) + 0x2000
    }

    /// Writes the full modern init sequence and asserts every step is accepted.
    pub(crate) fn init<D: VirtioDevice>(&self, t: &mut MmioTransport<D>, features: u64) {
        let mem = &self.mem;
        let mut w = |offset: u64, value: u64| {
            t.write(offset, W, value, mem)
                .unwrap_or_else(|violation| panic!("write {offset:#x}: {violation}"))
        };
        assert_eq!(w(REG_STATUS, 0), TransportEvent::Reset);
        w(REG_STATUS, u64::from(STATUS_ACKNOWLEDGE));
        w(REG_STATUS, u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER));
        w(REG_DRIVER_FEATURES_SEL, 0);
        w(REG_DRIVER_FEATURES, features & 0xffff_ffff);
        w(REG_DRIVER_FEATURES_SEL, 1);
        w(REG_DRIVER_FEATURES, features >> 32);
        w(
            REG_STATUS,
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK),
        );
        for (queue, size) in self.sizes.iter().enumerate() {
            let queue = u16::try_from(queue).expect("small");
            w(REG_QUEUE_SEL, u64::from(queue));
            w(REG_QUEUE_NUM, u64::from(*size));
            w(REG_QUEUE_DESC_LOW, Self::desc(queue));
            w(REG_QUEUE_DESC_HIGH, 0);
            w(REG_QUEUE_DRIVER_LOW, Self::avail(queue));
            w(REG_QUEUE_DRIVER_HIGH, 0);
            w(REG_QUEUE_DEVICE_LOW, Self::used(queue));
            w(REG_QUEUE_DEVICE_HIGH, 0);
            w(REG_QUEUE_READY, 1);
        }
        assert_eq!(
            w(
                REG_STATUS,
                u64::from(
                    STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK
                )
            ),
            TransportEvent::DriverOk
        );
    }

    /// Copies `bytes` into a fresh guest buffer and returns its address.
    pub(crate) fn alloc(&mut self, bytes: &[u8]) -> u64 {
        let addr = self.alloc_zeroed(u32::try_from(bytes.len()).expect("small"));
        self.mem
            .write_bytes(GuestAddress(addr), bytes)
            .expect("alloc");
        addr
    }

    /// Reserves a zeroed guest buffer, 16-byte aligned.
    pub(crate) fn alloc_zeroed(&mut self, len: u32) -> u64 {
        let addr = self.next_data;
        self.next_data += (u64::from(len.max(1)) + 15) & !15;
        assert!(
            self.next_data <= u64::try_from(MEM_LEN).expect("small"),
            "rig memory exhausted"
        );
        addr
    }

    /// Publishes one chain on `queue` and returns its head index.
    pub(crate) fn submit(&mut self, queue: u16, segments: &[Seg]) -> u16 {
        assert!(!segments.is_empty());
        let q = usize::from(queue);
        let size = self.sizes[q];
        let head = self.next_desc[q];
        let table = Self::desc(queue);
        for (offset, segment) in segments.iter().enumerate() {
            let index = (head + u16::try_from(offset).expect("small")) % size;
            let last = offset + 1 == segments.len();
            let mut flags = if segment.writable {
                VIRTQ_DESC_F_WRITE
            } else {
                0
            };
            if !last {
                flags |= VIRTQ_DESC_F_NEXT;
            }
            let descriptor = Descriptor {
                addr: segment.addr,
                len: segment.len,
                flags,
                next: (index + 1) % size,
            };
            self.mem
                .write_bytes(
                    GuestAddress(table + 16 * u64::from(index)),
                    &descriptor.to_bytes(),
                )
                .expect("desc");
        }
        self.next_desc[q] = (head + u16::try_from(segments.len()).expect("small")) % size;
        self.publish(queue, head);
        head
    }

    /// Places `head` in the available ring and bumps the index.
    pub(crate) fn publish(&mut self, queue: u16, head: u16) {
        let q = usize::from(queue);
        let slot = self.avail_idx[q] % self.sizes[q];
        let avail = Self::avail(queue);
        self.mem
            .write_obj_at(GuestAddress(avail + 4 + 2 * u64::from(slot)), head)
            .expect("ring");
        self.avail_idx[q] = self.avail_idx[q].wrapping_add(1);
        self.mem
            .write_obj_at(GuestAddress(avail + 2), self.avail_idx[q])
            .expect("idx");
    }

    pub(crate) fn notify<D: VirtioDevice>(
        &self,
        t: &mut MmioTransport<D>,
        queue: u16,
    ) -> TransportEvent {
        t.write(REG_QUEUE_NOTIFY, W, u64::from(queue), &self.mem)
            .expect("notify")
    }

    /// The used index the device published.
    pub(crate) fn used_idx(&self, queue: u16) -> u16 {
        self.mem
            .read_obj_at::<u16>(GuestAddress(Self::used(queue) + 2))
            .expect("used idx")
    }

    /// The `(head, len)` used element at ring position `slot`.
    pub(crate) fn used_elem(&self, queue: u16, slot: u16) -> (u32, u32) {
        let base = Self::used(queue) + 4 + 8 * u64::from(slot % self.sizes[usize::from(queue)]);
        let head = self
            .mem
            .read_obj_at::<u32>(GuestAddress(base))
            .expect("used");
        let len = self
            .mem
            .read_obj_at::<u32>(GuestAddress(base + 4))
            .expect("used");
        (head, len)
    }

    pub(crate) fn read(&self, addr: u64, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        self.mem
            .read_bytes(GuestAddress(addr), &mut buf)
            .expect("read");
        buf
    }

    /// Suppresses used-buffer interrupts on `queue`.
    pub(crate) fn set_no_interrupt(&self, queue: u16, suppress: bool) {
        self.mem
            .write_obj_at(GuestAddress(Self::avail(queue)), u16::from(suppress))
            .expect("flags");
    }
}
