//! Entropy device tests through the transport with a model driver.

use super::backend::{CounterEntropy, EntropyError};
use super::state::{RNG_STATE_LEN, RngState};
use super::*;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::{ServiceError, service_queue};
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::registers::AccessWidth;

fn boot(backend: CounterEntropy) -> (GuestRig, MmioTransport<RngDevice>) {
    let rig = GuestRig::new(&[16]);
    let mut t = MmioTransport::new(RngDevice::new(Box::new(backend))).expect("transport");
    rig.init(&mut t, RNG_FEATURES);
    (rig, t)
}

#[test]
fn fills_exactly_the_writable_capacity_across_segments() {
    let (mut rig, mut t) = boot(CounterEntropy::default());
    let first = rig.alloc_zeroed(10);
    let second = rig.alloc_zeroed(7);
    rig.submit(0, &[Seg::writable(first, 10), Seg::writable(second, 7)]);
    rig.notify(&mut t, 0);
    let report = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert_eq!((report.completed, report.interrupt), (1, true));
    assert_eq!(rig.used_elem(0, 0).1, 17);
    let mut expected: Vec<u8> = (0u8..17).collect();
    let mut got = rig.read(first, 10);
    got.extend(rig.read(second, 7));
    assert_eq!(got, expected);
    expected.clear();
    assert_eq!(rig.read(second + 7, 9), [0u8; 9], "nothing past the buffer");
    assert_eq!(
        t.device().counters(),
        RngCounters {
            filled: 1,
            bytes: 17,
            rejected: 0
        }
    );
}

#[test]
fn requests_above_the_limit_are_capped_and_readable_chains_are_rejected() {
    let (mut rig, mut t) = boot(CounterEntropy::default());
    let big = u32::try_from(MAX_ENTROPY_REQUEST + 4096).expect("small");
    let buffer = rig.alloc_zeroed(big);
    rig.submit(0, &[Seg::writable(buffer, big)]);
    let readable = rig.alloc(&[1, 2, 3, 4]);
    let sink = rig.alloc_zeroed(64);
    rig.submit(0, &[Seg::readable(readable, 4), Seg::writable(sink, 64)]);
    let report = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert_eq!(report.completed, 2);
    let capped = u32::try_from(MAX_ENTROPY_REQUEST).expect("small");
    assert_eq!(rig.used_elem(0, 0).1, capped);
    assert_eq!(rig.used_elem(0, 1).1, 0);
    assert_eq!(rig.read(buffer + u64::from(capped), 64), [0u8; 64]);
    assert_eq!(rig.read(sink, 64), [0u8; 64], "rejected chain untouched");
    let counters = t.device().counters();
    assert_eq!((counters.filled, counters.rejected), (1, 1));
    assert_eq!(counters.bytes, u64::from(capped));
}

#[test]
fn host_entropy_failure_stops_the_device_with_a_typed_fault() {
    let backend = CounterEntropy {
        fail: Some(EntropyError::Short),
        ..CounterEntropy::default()
    };
    let (mut rig, mut t) = boot(backend);
    let buffer = rig.alloc_zeroed(32);
    rig.submit(0, &[Seg::writable(buffer, 32)]);
    assert_eq!(
        service_queue(&mut t, &rig.mem, 0, 8),
        Err(ServiceError::Fault(DeviceFault::Backend))
    );
    assert!(!t.is_active());
    assert_eq!(rig.read(buffer, 32), [0u8; 32]);
    assert_eq!(rig.used_idx(0), 0);
}

#[test]
fn config_space_is_empty_and_state_is_identity_only() {
    let (rig, mut t) = boot(CounterEntropy::default());
    assert!(t.read(0x100, AccessWidth::U8).is_err());
    assert!(t.write(0x100, AccessWidth::U8, 1, &rig.mem).is_err());
    let raw = t.device().snapshot_state();
    assert_eq!(raw.len(), RNG_STATE_LEN);
    assert_eq!(
        RngState::from_bytes(&raw)
            .expect("decode")
            .to_bytes()
            .to_vec(),
        raw
    );
    let mut fresh = RngDevice::new(Box::new(CounterEntropy::default()));
    assert_eq!(fresh.restore_state(&raw), Ok(()));
    assert_eq!(
        fresh.restore_state(&raw[1..]),
        Err(DeviceStateError::Malformed)
    );
    let mut wrong_id = raw.clone();
    wrong_id[1] = 5;
    assert_eq!(
        fresh.restore_state(&wrong_id),
        Err(DeviceStateError::Incompatible)
    );
    let mut wrong_features = raw.clone();
    wrong_features[5] = 1;
    assert_eq!(
        fresh.restore_state(&wrong_features),
        Err(DeviceStateError::Incompatible)
    );
    let mut wrong_version = raw;
    wrong_version[0] = 9;
    assert_eq!(
        fresh.restore_state(&wrong_version),
        Err(DeviceStateError::Malformed)
    );
}

#[test]
fn random_chain_shapes_never_panic_and_never_exceed_capacity() {
    let (mut rig, mut t) = boot(CounterEntropy::default());
    let mut seed = 0x0123_4567_89ab_cdefu64;
    for _ in 0..200 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let count = usize::try_from(1 + seed % 4).expect("small");
        let mut segments = Vec::new();
        let mut total = 0u32;
        for index in 0..count {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let len = u32::try_from(1 + (seed >> 33) % 3000).expect("small");
            let addr = rig.alloc_zeroed(len + 16);
            let writable = index != 0 || !seed.is_multiple_of(5);
            segments.push(Seg {
                addr,
                len,
                writable,
            });
            total += len;
        }
        rig.submit(0, &segments);
        let before = rig.used_idx(0);
        let report = service_queue(&mut t, &rig.mem, 0, 4).expect("never faults");
        assert_eq!(report.completed + report.rejected, 1);
        if report.completed == 1 {
            assert!(rig.used_elem(0, before).1 <= total);
        }
        assert!(t.is_active());
    }
}

#[cfg(unix)]
#[test]
fn os_entropy_fills_buffers_with_fresh_bytes() {
    use super::backend::{EntropyBackend, OsEntropy};
    let mut source = OsEntropy::open().expect("urandom");
    let mut first = [0u8; 64];
    let mut second = [0u8; 64];
    source.fill(&mut first).expect("fill");
    source.fill(&mut second).expect("fill");
    assert_ne!(first, [0u8; 64]);
    assert_ne!(first, second);
    let mut empty = [0u8; 0];
    source.fill(&mut empty).expect("empty fill");
}
