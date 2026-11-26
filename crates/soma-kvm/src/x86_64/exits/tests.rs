//! Unit proofs for the exit ledger: distinct classes, one recorded first pass, and the
//! sampling limit that bounds what the instrument costs.

use super::{ExitLedger, ExitReason, GuestEnd, SAMPLE_LIMIT};

#[test]
fn every_class_has_a_distinct_index_and_name() {
    let mut names: Vec<&str> = ExitReason::ALL.iter().map(|reason| reason.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ExitReason::ALL.len());
    for (index, reason) in ExitReason::ALL.into_iter().enumerate() {
        assert_eq!(reason.index(), index);
    }
}

#[test]
fn an_untouched_ledger_reports_nothing_and_no_first_run() {
    let ledger = ExitLedger::new();
    let counts = ledger.counts();
    assert_eq!(counts.total(), 0);
    assert_eq!(counts.first_run_ns, 0);
    assert_eq!(counts.sampled, 0);
    assert!(ledger.first_entry().is_none());
    assert!(ledger.first_return().is_none());
}

#[test]
fn one_pass_records_a_first_entry_a_first_return_and_one_count() {
    let ledger = ExitLedger::new();
    let mut sampler = ledger.sampler();
    sampler.entering();
    sampler.returned(&Ok(kvm_ioctls::VcpuExit::Hlt));
    let counts = ledger.counts();
    assert_eq!(counts.of(ExitReason::Halt), 1);
    assert_eq!(counts.total(), 1);
    assert_eq!(counts.sampled, 1);
    let entry = ledger.first_entry().expect("a first entry");
    let ret = ledger.first_return().expect("a first return");
    assert!(ret >= entry);
    assert!(counts.inside_ns >= counts.first_run_ns.saturating_sub(1));
}

#[test]
fn sampling_stops_at_the_limit_while_counting_continues() {
    let ledger = ExitLedger::new();
    let mut sampler = ledger.sampler();
    for _ in 0..(SAMPLE_LIMIT + 8) {
        sampler.entering();
        sampler.returned(&Ok(kvm_ioctls::VcpuExit::Intr));
    }
    let counts = ledger.counts();
    assert_eq!(counts.sampled, SAMPLE_LIMIT);
    assert_eq!(counts.of(ExitReason::Interrupted), SAMPLE_LIMIT + 8);
    assert_eq!(counts.total(), SAMPLE_LIMIT + 8);
}

#[test]
fn the_first_entry_and_return_are_never_overwritten() {
    let ledger = ExitLedger::new();
    let mut sampler = ledger.sampler();
    sampler.entering();
    sampler.returned(&Ok(kvm_ioctls::VcpuExit::Hlt));
    let first = ledger.first_entry().expect("a first entry");
    let first_return = ledger.first_return().expect("a first return");
    sampler.entering();
    sampler.returned(&Ok(kvm_ioctls::VcpuExit::Hlt));
    assert_eq!(ledger.first_entry(), Some(first));
    assert_eq!(ledger.first_return(), Some(first_return));
}

/// A guest's own end is kept, and only the first: a second vCPU's exit cannot change it.
#[test]
fn the_first_guest_end_is_the_one_recorded() {
    let ledger = ExitLedger::new();
    assert_eq!(ledger.guest_end(), None);
    ledger.record_guest_end(GuestEnd::PowerOff);
    ledger.record_guest_end(GuestEnd::Reboot);
    assert_eq!(ledger.guest_end(), Some(GuestEnd::PowerOff));
}

#[test]
fn only_a_reset_and_a_power_off_are_ends_the_guest_asked_for() {
    use crate::x86_64::GuestExit;
    assert_eq!(GuestExit::Reset.guest_end(), Some(GuestEnd::Reboot));
    assert_eq!(GuestExit::PowerOff.guest_end(), Some(GuestEnd::PowerOff));
    for other in [
        GuestExit::Halt,
        GuestExit::Shutdown,
        GuestExit::Sentinel,
        GuestExit::Paused,
    ] {
        assert_eq!(other.guest_end(), None);
    }
}
