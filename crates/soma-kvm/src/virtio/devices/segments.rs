//! Bounded copies between validated descriptor-chain segments and host buffers.
//!
//! The chain was validated by `walk_chain`, so every segment lies inside
//! registered guest memory; the copies here still go through the checked
//! [`GuestMemory`] seam and never trust a length twice.

use crate::virtio::guest_memory::{GuestAddress, GuestMemory, GuestMemoryError};
use crate::virtio::queue::chain::{ChainSegment, DescriptorChain};

/// Copies up to `buf.len()` device-readable bytes, skipping the first `skip`.
///
/// Returns how many bytes were copied, which is less than `buf.len()` only
/// when the readable area is exhausted.
///
/// # Errors
/// Forwards a guest-memory rejection; the chain is then considered hostile.
pub fn read_readable<M: GuestMemory + ?Sized>(
    mem: &M,
    chain: &DescriptorChain,
    skip: u64,
    buf: &mut [u8],
) -> Result<usize, GuestMemoryError> {
    let len = buf.len();
    span(chain.readable(), skip, len, |addr, start, end| {
        mem.read_bytes(addr, &mut buf[start..end])
    })
}

/// Copies `bytes` into the device-writable area, skipping the first `skip`.
///
/// Returns how many bytes were written, which is less than `bytes.len()` only
/// when the writable capacity is exhausted.
///
/// # Errors
/// Forwards a guest-memory rejection; the chain is then considered hostile.
pub fn write_writable<M: GuestMemory + ?Sized>(
    mem: &M,
    chain: &DescriptorChain,
    skip: u64,
    bytes: &[u8],
) -> Result<usize, GuestMemoryError> {
    span(chain.writable(), skip, bytes.len(), |addr, start, end| {
        mem.write_bytes(addr, &bytes[start..end])
    })
}

/// Walks `segments`, skipping `skip` bytes, and calls `copy` for each host
/// byte range `[start, end)` paired with its guest address until `len` bytes
/// are covered or the segments run out.
fn span<'a>(
    segments: impl Iterator<Item = &'a ChainSegment>,
    mut skip: u64,
    len: usize,
    mut copy: impl FnMut(GuestAddress, usize, usize) -> Result<(), GuestMemoryError>,
) -> Result<usize, GuestMemoryError> {
    let mut done = 0usize;
    for segment in segments {
        if done == len {
            break;
        }
        let segment_len = u64::from(segment.len);
        if skip >= segment_len {
            skip -= segment_len;
            continue;
        }
        let start = skip;
        skip = 0;
        let remaining = u64::try_from(len - done).unwrap_or(u64::MAX);
        let take = (segment_len - start).min(remaining);
        let take_usize = usize::try_from(take).map_err(|_| GuestMemoryError::Overflow {
            addr: segment.addr,
            len: take,
        })?;
        let addr = segment
            .addr
            .checked_add(start)
            .ok_or(GuestMemoryError::Overflow {
                addr: segment.addr,
                len: start,
            })?;
        copy(addr, done, done + take_usize)?;
        done += take_usize;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::guest_memory::VecGuestMemory;
    use crate::virtio::queue::chain::{
        ChainLimits, Descriptor, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE, walk_chain,
    };

    fn chain(mem: &VecGuestMemory) -> DescriptorChain {
        let table = GuestAddress(0x100);
        let descriptors = [
            Descriptor {
                addr: 0x1000,
                len: 3,
                flags: VIRTQ_DESC_F_NEXT,
                next: 1,
            },
            Descriptor {
                addr: 0x2000,
                len: 4,
                flags: VIRTQ_DESC_F_NEXT,
                next: 2,
            },
            Descriptor {
                addr: 0x3000,
                len: 2,
                flags: VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
                next: 3,
            },
            Descriptor {
                addr: 0x4000,
                len: 5,
                flags: VIRTQ_DESC_F_WRITE,
                next: 0,
            },
        ];
        for (index, descriptor) in descriptors.iter().enumerate() {
            let addr = GuestAddress(0x100 + 16 * index as u64);
            mem.write_bytes(addr, &descriptor.to_bytes()).expect("desc");
        }
        mem.write_bytes(GuestAddress(0x1000), b"abc").expect("data");
        mem.write_bytes(GuestAddress(0x2000), b"defg")
            .expect("data");
        let limits = ChainLimits {
            max_descriptors: 8,
            max_bytes: 64,
        };
        walk_chain(mem, table, 8, 0, limits).expect("chain")
    }

    #[test]
    fn readable_copy_spans_segments_and_honors_skip() {
        let mem = VecGuestMemory::flat(0x8000).expect("memory");
        let chain = chain(&mem);
        let mut buf = [0u8; 16];
        assert_eq!(read_readable(&mem, &chain, 0, &mut buf), Ok(7));
        assert_eq!(&buf[..7], b"abcdefg");
        let mut buf = [0u8; 3];
        assert_eq!(read_readable(&mem, &chain, 2, &mut buf), Ok(3));
        assert_eq!(&buf, b"cde");
        assert_eq!(read_readable(&mem, &chain, 7, &mut buf), Ok(0));
        assert_eq!(read_readable(&mem, &chain, u64::MAX, &mut buf), Ok(0));
    }

    #[test]
    fn writable_copy_spans_segments_and_stops_at_capacity() {
        let mem = VecGuestMemory::flat(0x8000).expect("memory");
        let chain = chain(&mem);
        assert_eq!(write_writable(&mem, &chain, 0, b"0123456789"), Ok(7));
        let mut first = [0u8; 2];
        let mut second = [0u8; 5];
        mem.read_bytes(GuestAddress(0x3000), &mut first)
            .expect("read");
        mem.read_bytes(GuestAddress(0x4000), &mut second)
            .expect("read");
        assert_eq!(&first, b"01");
        assert_eq!(&second, b"23456");
        assert_eq!(write_writable(&mem, &chain, 6, b"Z"), Ok(1));
        mem.read_bytes(GuestAddress(0x4000), &mut second)
            .expect("read");
        assert_eq!(&second, b"2345Z");
        assert_eq!(write_writable(&mem, &chain, 0, &[]), Ok(0));
    }
}
