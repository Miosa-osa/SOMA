#![no_main]

//! Drives the block, network, and vsock request parsers with a hostile
//! descriptor table and guest memory built from the fuzz input.

use libfuzzer_sys::fuzz_target;
use soma_kvm::{
    ChainLimits, GuestAddress, GuestMemory, RequestLimits, VecGuestMemory, parse_request,
    parse_tx, validate_tx, walk_chain,
};

const MEMORY_LEN: usize = 1 << 16;
const TABLE: u64 = 0x100;

fuzz_target!(|input: &[u8]| {
    let mem = VecGuestMemory::flat(MEMORY_LEN).expect("memory");
    let copy = input.len().min(MEMORY_LEN);
    mem.write_bytes(GuestAddress(0), &input[..copy]).expect("fill");
    let limits = ChainLimits {
        max_descriptors: 16,
        max_bytes: 1 << 20,
    };
    let head = u16::from(input.first().copied().unwrap_or(0)) % 16;
    let Ok(chain) = walk_chain(&mem, GuestAddress(TABLE), 16, head, limits) else {
        return;
    };
    let block = RequestLimits {
        capacity_bytes: 1 << 20,
        read_only: input.len() % 2 == 0,
        flush: input.len() % 3 == 0,
    };
    let _ = parse_request(&mem, &chain, block);
    let _ = validate_tx(&mem, &chain);
    let _ = parse_tx(&mem, &chain, 1234);
});
