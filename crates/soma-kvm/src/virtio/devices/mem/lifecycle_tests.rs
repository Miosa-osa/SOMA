//! A guest that reboots, a host that changes its mind between requests, the configuration the
//! driver reads, and the snapshot record: none of them may leave the device holding memory the
//! guest no longer knows about.

use super::rig::{ACK, NACK, PLUG, REGION, Rig, UNPLUG};
use super::state::MEM_STATE_LEN;
use super::*;
use crate::virtio::devices::harness::GuestRig;
use crate::virtio::transport::registers::{
    AccessWidth, CONFIG_OFFSET, REG_CONFIG_GENERATION, REG_INTERRUPT_ACK, REG_INTERRUPT_STATUS,
};

const BLOCKS: u64 = 16;

/// A new driver instance: a fresh queue and memory image, initialised through the status
/// register, which is what a rebooted guest does.
fn reboot(r: &mut Rig) {
    r.guest = GuestRig::new(&[16]);
    r.guest.init(&mut r.t, MEM_FEATURES);
}

fn config(r: &mut Rig, offset: u64) -> u64 {
    let low =
        r.t.read(CONFIG_OFFSET + offset, AccessWidth::U32)
            .expect("low");
    let high =
        r.t.read(CONFIG_OFFSET + offset + 4, AccessWidth::U32)
            .expect("high");
    low | high << 32
}

#[test]
fn a_rebooted_guest_has_forgotten_every_block_and_the_host_gets_them_back() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    reboot(&mut r);
    assert_eq!(r.plugged(), 0);
    assert!(r.t.device().is_quiescent());
    assert_eq!(r.discards(), vec![(0, BLOCKS * BLOCK_SIZE)]);
    assert_eq!(config(&mut r, 40), 0, "the driver reads nothing plugged");
    // The host's request outlives the reboot, so the new driver can plug it again.
    assert_eq!(config(&mut r, 48), 4 * BLOCK_SIZE);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
}

#[test]
fn rebooting_an_idle_guest_costs_the_host_nothing() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    reboot(&mut r);
    reboot(&mut r);
    assert!(r.discards().is_empty());
}

#[test]
fn a_request_published_but_never_serviced_dies_with_the_reboot() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let body = r.guest.alloc(&super::rig::encode(PLUG, Rig::at(0), 4));
    let reply = r.guest.alloc_zeroed(10);
    r.guest.submit(
        0,
        &[
            crate::virtio::devices::harness::Seg::readable(body, 24),
            crate::virtio::devices::harness::Seg::writable(reply, 10),
        ],
    );
    reboot(&mut r);
    let report = r.serve().expect("service");
    assert_eq!(report.completed, 0, "the old chain is not in the new ring");
    assert_eq!(r.plugged(), 0);
}

#[test]
fn the_host_changing_its_mind_between_requests_is_always_honoured_in_order() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    r.want(2);
    assert_eq!(r.ask(UNPLUG, Rig::at(2), 2).kind, ACK);
    assert_eq!(r.ask(PLUG, Rig::at(2), 1).kind, NACK, "2 held, 2 asked");
    r.want(6);
    assert_eq!(r.ask(PLUG, Rig::at(2), 4).kind, ACK);
    assert_eq!(r.plugged(), 6);
    r.want(0);
    assert_eq!(r.ask(PLUG, Rig::at(8), 1).kind, NACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 6).kind, ACK);
    assert!(r.t.device().is_quiescent());
}

#[test]
fn the_driver_reads_the_region_and_the_progress_through_the_registers() {
    let mut r = Rig::new(BLOCKS);
    assert_eq!(config(&mut r, 0), BLOCK_SIZE);
    assert_eq!(config(&mut r, 16), REGION);
    assert_eq!(config(&mut r, 24), BLOCKS * BLOCK_SIZE);
    assert_eq!(
        config(&mut r, 32),
        BLOCKS * BLOCK_SIZE,
        "usable equals the region"
    );
    let generation =
        r.t.read(REG_CONFIG_GENERATION, AccessWidth::U32)
            .expect("gen");
    r.want(3);
    assert_ne!(
        r.t.read(REG_CONFIG_GENERATION, AccessWidth::U32),
        Ok(generation)
    );
    assert_eq!(config(&mut r, 48), 3 * BLOCK_SIZE);
    assert_eq!(r.ask(PLUG, Rig::at(0), 3).kind, ACK);
    assert_eq!(
        config(&mut r, 40),
        3 * BLOCK_SIZE,
        "plugged follows the device"
    );
    assert_eq!(config(&mut r, 48), 3 * BLOCK_SIZE);
}

#[test]
fn the_config_change_interrupt_is_raised_by_the_host_and_cleared_by_the_driver() {
    let mut r = Rig::new(BLOCKS);
    let before =
        r.t.read(REG_INTERRUPT_STATUS, AccessWidth::U32)
            .expect("status");
    r.want(1);
    let raised =
        r.t.read(REG_INTERRUPT_STATUS, AccessWidth::U32)
            .expect("status");
    assert_eq!(raised & 2, 2, "config-change bit");
    assert_eq!(before & 2, 0);
    r.t.write(REG_INTERRUPT_ACK, AccessWidth::U32, 2, &r.guest.mem)
        .expect("ack");
    assert_eq!(
        r.t.read(REG_INTERRUPT_STATUS, AccessWidth::U32)
            .expect("status")
            & 2,
        0
    );
}

#[test]
fn the_driver_cannot_write_the_configuration() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    for offset in [0_u64, 16, 24, 32, 40, 48, 52] {
        let _ = r.t.write(
            CONFIG_OFFSET + offset,
            AccessWidth::U32,
            0xffff_ffff,
            &r.guest.mem,
        );
    }
    assert_eq!(config(&mut r, 0), BLOCK_SIZE);
    assert_eq!(config(&mut r, 24), BLOCKS * BLOCK_SIZE);
    assert_eq!(config(&mut r, 40), 0);
    assert_eq!(config(&mut r, 48), 2 * BLOCK_SIZE);
    assert_eq!(r.t.device().requested_size(), 2 * BLOCK_SIZE);
}

#[test]
fn a_snapshot_record_never_depends_on_what_is_plugged_or_requested() {
    let mut r = Rig::new(BLOCKS);
    let empty = r.t.device().snapshot_state();
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    assert_eq!(r.t.device().snapshot_state(), empty);
    assert_eq!(empty.len(), MEM_STATE_LEN);
}

#[test]
fn every_damaged_record_is_refused_and_leaves_the_device_as_it_was() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, ACK);
    let good = r.t.device().snapshot_state();
    for len in (0..=MEM_STATE_LEN + 8).filter(|len| *len != MEM_STATE_LEN) {
        let mut raw = good.clone();
        raw.resize(len, 0);
        assert_eq!(
            r.t.device_mut().restore_state(&raw),
            Err(DeviceStateError::Malformed),
            "length {len}"
        );
    }
    for byte in 0..MEM_STATE_LEN {
        for flip in [0x01_u8, 0x80, 0xff] {
            let mut raw = good.clone();
            raw[byte] ^= flip;
            assert!(
                r.t.device_mut().restore_state(&raw).is_err(),
                "byte {byte} ^ {flip:#x} was accepted"
            );
        }
    }
    assert_eq!(r.plugged(), 2, "a refused record did not empty the region");
    assert_eq!(r.t.device().requested_size(), 4 * BLOCK_SIZE);
    r.t.device_mut()
        .restore_state(&good)
        .expect("the real record");
    assert!(r.t.device().is_quiescent());
}
