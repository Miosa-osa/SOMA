//! The memory leg of a live resize: the order things happen in, and what is reported.
//!
//! Memory is added by asking the device to plug blocks and waiting for the guest to do it, and
//! given back by asking it to unplug and waiting again. The guest may plug less than asked, or
//! refuse to give memory back, so the answer is the plugged amount the device reports after the
//! wait, never the amount requested. A jailed machine also has a `memory.max` that must sit above
//! the new total before a plug and may only come down after the memory is really gone.

use std::time::Duration;

use soma_vmm::sandbox::{MemoryError, MemoryLevel, Session, SessionError};

use crate::{DimensionState, MemoryResize};

const MIB: u64 = 1024 * 1024;

/// How often the device is read, and how long each direction may take.
pub(super) struct Timing {
    pub(super) interval: Duration,
    pub(super) grow: Duration,
    pub(super) shrink: Duration,
}

/// The timings a real resize waits with.
pub(super) const REAL: Timing = Timing {
    interval: Duration::from_millis(50),
    grow: Duration::from_secs(10),
    shrink: Duration::from_secs(30),
};

/// Memory the machine host needs beyond the guest's own RAM, for a jailed machine's `memory.max`.
pub(super) const HOST_OVERHEAD_BYTES: u64 = 512 * MIB;

/// Why the memory device could not be reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LinkError {
    Device(MemoryError),
    Session,
}

impl From<SessionError> for LinkError {
    fn from(_: SessionError) -> Self {
        Self::Session
    }
}

/// What a resize needs from a machine: its memory device and, if it has one, its cgroup limit.
pub(super) trait MemoryLink {
    fn set_requested(&mut self, bytes: u64) -> Result<MemoryLevel, LinkError>;
    fn level(&mut self) -> Result<MemoryLevel, LinkError>;
    /// Sets the machine's `memory.max`. A machine with no such leaf accepts and does nothing.
    fn set_memory_max(&mut self, bytes: u64) -> Result<(), ()>;
}

/// A machine this process holds on its own sandbox thread.
pub(super) struct ResidentLink<'a>(&'a mut Session);

impl<'a> ResidentLink<'a> {
    pub(super) fn new(session: &'a mut Session) -> Self {
        Self(session)
    }
}

impl MemoryLink for ResidentLink<'_> {
    fn set_requested(&mut self, bytes: u64) -> Result<MemoryLevel, LinkError> {
        self.0
            .set_requested_memory(bytes)?
            .map_err(LinkError::Device)
    }

    fn level(&mut self) -> Result<MemoryLevel, LinkError> {
        self.0.memory()?.map_err(LinkError::Device)
    }

    // An unjailed host has no memory leaf; its machine is bounded by the host's own cgroup.
    fn set_memory_max(&mut self, _bytes: u64) -> Result<(), ()> {
        Ok(())
    }
}

/// What a requested total means for a machine.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Plan {
    NoDevice,
    Reject(&'static str),
    Same,
    Grow { plugged_mb: u64 },
    Shrink { plugged_mb: u64 },
}

/// Decides, before anything is written, what `wanted_mb` total means.
pub(super) fn plan(booted_mb: u64, ceiling_mb: u64, plugged_mb: u64, wanted_mb: u64) -> Plan {
    if ceiling_mb <= booted_mb {
        return Plan::NoDevice;
    }
    if wanted_mb < booted_mb {
        return Plan::Reject("below_booted");
    }
    if wanted_mb > ceiling_mb {
        return Plan::Reject("above_ceiling");
    }
    let target = wanted_mb - booted_mb;
    match target.cmp(&plugged_mb) {
        std::cmp::Ordering::Equal => Plan::Same,
        std::cmp::Ordering::Greater => Plan::Grow { plugged_mb: target },
        std::cmp::Ordering::Less => Plan::Shrink { plugged_mb: target },
    }
}

fn answer(
    state: DimensionState,
    booted_mb: u64,
    ceiling_mb: u64,
    plugged_mb: u64,
    requested_mb: Option<u64>,
    reason: Option<&str>,
) -> MemoryResize {
    MemoryResize {
        state,
        requested_mb,
        plugged_mb,
        total_mb: booted_mb + plugged_mb,
        booted_mb,
        ceiling_mb,
        reason: reason.map(ToOwned::to_owned),
    }
}

/// Nothing was asked: the machine as it is.
pub(super) fn report_unchanged(booted_mb: u64, ceiling_mb: u64, plugged_mb: u64) -> MemoryResize {
    answer(
        DimensionState::Unchanged,
        booted_mb,
        ceiling_mb,
        plugged_mb,
        None,
        None,
    )
}

/// Nothing was asked: the memory the device holds now.
///
/// A launch may have plugged memory beyond the Generation's boot size, so a machine that was never
/// resized live can still hold more than it booted with. The answer is the device's level, never
/// the boot size assumed.
pub(super) fn current(link: &mut impl MemoryLink, booted_mb: u64) -> MemoryResize {
    match link.level() {
        Ok(level) => answer(
            DimensionState::Unchanged,
            booted_mb,
            booted_mb + level.region_bytes / MIB,
            level.plugged_bytes / MIB,
            None,
            None,
        ),
        Err(LinkError::Device(MemoryError::NoDevice)) => report_unchanged(booted_mb, booted_mb, 0),
        Err(error) => failed(booted_mb, booted_mb, 0, None, error),
    }
}

/// The machine cannot do it live, for a reason that does not depend on the request.
pub(super) fn unsupported(booted_mb: u64, requested_mb: Option<u64>, reason: &str) -> MemoryResize {
    answer(
        DimensionState::Unsupported,
        booted_mb,
        booted_mb,
        0,
        requested_mb,
        Some(reason),
    )
}

/// Polls the device until `done` holds or the allowance runs out, and returns the last level.
fn settle(
    link: &mut impl MemoryLink,
    allowance: Duration,
    timing: &Timing,
    sleep: &mut dyn FnMut(Duration),
    done: impl Fn(&MemoryLevel) -> bool,
) -> Result<MemoryLevel, LinkError> {
    let mut level = link.level()?;
    let polls = allowance.as_millis() / timing.interval.as_millis().max(1);
    for _ in 0..polls {
        if done(&level) {
            break;
        }
        sleep(timing.interval);
        level = link.level()?;
    }
    Ok(level)
}

/// Resizes the machine's memory to `wanted_mb` in total and says what the device then holds.
pub(super) fn resize(
    link: &mut impl MemoryLink,
    booted_mb: u64,
    wanted_mb: u64,
    timing: &Timing,
    sleep: &mut dyn FnMut(Duration),
) -> MemoryResize {
    let wanted = Some(wanted_mb);
    let start = match link.level() {
        Ok(level) => level,
        Err(LinkError::Device(MemoryError::NoDevice)) => {
            return unsupported(booted_mb, wanted, "generation_without_virtio_mem");
        }
        Err(error) => return failed(booted_mb, booted_mb, 0, wanted, error),
    };
    let ceiling_mb = booted_mb + start.region_bytes / MIB;
    let plugged_mb = start.plugged_bytes / MIB;
    let say = |state, plugged: u64, reason: Option<&str>| {
        answer(state, booted_mb, ceiling_mb, plugged, wanted, reason)
    };
    match plan(booted_mb, ceiling_mb, plugged_mb, wanted_mb) {
        Plan::NoDevice => say(
            DimensionState::Unsupported,
            0,
            Some("generation_without_virtio_mem"),
        ),
        Plan::Reject(reason) => say(DimensionState::Unsupported, plugged_mb, Some(reason)),
        Plan::Same => say(DimensionState::Unchanged, plugged_mb, None),
        Plan::Grow { plugged_mb: target } => grow(
            link,
            (booted_mb, ceiling_mb, wanted_mb, target),
            (timing, sleep),
            plugged_mb,
        ),
        Plan::Shrink { plugged_mb: target } => shrink(
            link,
            (booted_mb, ceiling_mb, wanted_mb, target),
            (timing, sleep),
            plugged_mb,
        ),
    }
}

fn grow(
    link: &mut impl MemoryLink,
    sizes: (u64, u64, u64, u64),
    wait: (&Timing, &mut dyn FnMut(Duration)),
    before_mb: u64,
) -> MemoryResize {
    let (booted_mb, ceiling_mb, wanted_mb, target_mb) = sizes;
    let (timing, sleep) = wait;
    let say = |state, plugged: u64, reason: Option<&str>| {
        answer(
            state,
            booted_mb,
            ceiling_mb,
            plugged,
            Some(wanted_mb),
            reason,
        )
    };
    // The limit goes up first: plugged pages must never meet the old one.
    if link
        .set_memory_max(wanted_mb * MIB + HOST_OVERHEAD_BYTES)
        .is_err()
    {
        return say(
            DimensionState::Failed,
            before_mb,
            Some("cgroup_write_failed"),
        );
    }
    if let Err(error) = link.set_requested(target_mb * MIB) {
        return failed(booted_mb, ceiling_mb, before_mb, Some(wanted_mb), error);
    }
    let goal = target_mb * MIB;
    match settle(link, timing.grow, timing, sleep, |level| {
        level.plugged_bytes >= goal
    }) {
        Ok(level) if level.plugged_bytes >= goal => {
            say(DimensionState::AppliedLive, level.plugged_bytes / MIB, None)
        }
        Ok(level) => say(
            DimensionState::Partial,
            level.plugged_bytes / MIB,
            Some("guest_did_not_plug"),
        ),
        Err(error) => failed(booted_mb, ceiling_mb, before_mb, Some(wanted_mb), error),
    }
}

fn shrink(
    link: &mut impl MemoryLink,
    sizes: (u64, u64, u64, u64),
    wait: (&Timing, &mut dyn FnMut(Duration)),
    before_mb: u64,
) -> MemoryResize {
    let (booted_mb, ceiling_mb, wanted_mb, target_mb) = sizes;
    let (timing, sleep) = wait;
    let say = |state, plugged: u64, reason: Option<&str>| {
        answer(
            state,
            booted_mb,
            ceiling_mb,
            plugged,
            Some(wanted_mb),
            reason,
        )
    };
    if let Err(error) = link.set_requested(target_mb * MIB) {
        return failed(booted_mb, ceiling_mb, before_mb, Some(wanted_mb), error);
    }
    let goal = target_mb * MIB;
    let level = match settle(link, timing.shrink, timing, sleep, |level| {
        level.plugged_bytes <= goal
    }) {
        Ok(level) => level,
        Err(error) => return failed(booted_mb, ceiling_mb, before_mb, Some(wanted_mb), error),
    };
    // The limit comes down only to what the guest really holds, and only now.
    let held_mb = level.plugged_bytes / MIB;
    let lowered = link
        .set_memory_max((booted_mb + held_mb) * MIB + HOST_OVERHEAD_BYTES)
        .is_ok();
    match (level.plugged_bytes <= goal, lowered) {
        (true, true) => say(DimensionState::AppliedLive, held_mb, None),
        (true, false) => say(
            DimensionState::AppliedLive,
            held_mb,
            Some("memory_limit_not_lowered"),
        ),
        (false, _) => say(
            DimensionState::Partial,
            held_mb,
            Some("guest_could_not_offline"),
        ),
    }
}

fn failed(
    booted_mb: u64,
    ceiling_mb: u64,
    plugged_mb: u64,
    requested_mb: Option<u64>,
    error: LinkError,
) -> MemoryResize {
    let reason = match error {
        LinkError::Device(MemoryError::Busy) => "checkpoint_in_flight",
        LinkError::Device(MemoryError::BadSize) => "bad_size",
        LinkError::Device(MemoryError::NoDevice) => "generation_without_virtio_mem",
        LinkError::Session => "session_ended",
    };
    answer(
        DimensionState::Failed,
        booted_mb,
        ceiling_mb,
        plugged_mb,
        requested_mb,
        Some(reason),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_total_between_boot_and_ceiling_is_a_grow_or_a_shrink_of_the_plugged_part() {
        assert_eq!(plan(512, 2048, 0, 1024), Plan::Grow { plugged_mb: 512 });
        assert_eq!(
            plan(512, 2048, 1024, 1024),
            Plan::Shrink { plugged_mb: 512 }
        );
        assert_eq!(plan(512, 2048, 512, 1024), Plan::Same);
        assert_eq!(plan(512, 2048, 0, 512), Plan::Same);
        assert_eq!(plan(512, 2048, 0, 511), Plan::Reject("below_booted"));
        assert_eq!(plan(512, 2048, 0, 2049), Plan::Reject("above_ceiling"));
        assert_eq!(plan(512, 512, 0, 1024), Plan::NoDevice);
    }

    /// A device whose guest plugs `step` bytes per read until it reaches what was asked.
    struct Fake {
        region: u64,
        requested: u64,
        plugged: u64,
        step: u64,
        stuck_above: Option<u64>,
        refuse: Option<MemoryError>,
        max_writes: Vec<u64>,
        order: Vec<&'static str>,
        max_fails: bool,
    }

    impl Fake {
        fn new(region_mb: u64, plugged_mb: u64) -> Self {
            Self {
                region: region_mb * MIB,
                requested: plugged_mb * MIB,
                plugged: plugged_mb * MIB,
                step: 64 * MIB,
                stuck_above: None,
                refuse: None,
                max_writes: Vec::new(),
                order: Vec::new(),
                max_fails: false,
            }
        }

        fn level_now(&self) -> MemoryLevel {
            MemoryLevel {
                requested_bytes: self.requested,
                plugged_bytes: self.plugged,
                region_bytes: self.region,
            }
        }
    }

    #[test]
    fn an_untouched_machine_reports_what_the_device_holds_not_what_it_booted_with() {
        // A launch that plugged memory leaves a machine that was never resized live holding more
        // than it booted with, and the answer must say so.
        let mut link = Fake::new(8192, 8192);
        let held = current(&mut link, 8192);
        assert_eq!(held.state, DimensionState::Unchanged);
        assert_eq!(
            (
                held.booted_mb,
                held.plugged_mb,
                held.total_mb,
                held.ceiling_mb
            ),
            (8192, 8192, 16384, 16384)
        );
        assert_eq!(held.requested_mb, None);
    }

    impl MemoryLink for Fake {
        fn set_requested(&mut self, bytes: u64) -> Result<MemoryLevel, LinkError> {
            if let Some(error) = self.refuse {
                return Err(LinkError::Device(error));
            }
            self.order.push("request");
            self.requested = bytes;
            Ok(self.level_now())
        }

        fn level(&mut self) -> Result<MemoryLevel, LinkError> {
            if let Some(error) = self.refuse {
                return Err(LinkError::Device(error));
            }
            let floor = self.stuck_above.unwrap_or(0);
            if self.plugged < self.requested {
                self.plugged = (self.plugged + self.step).min(self.requested);
            } else if self.plugged > self.requested {
                self.plugged = self
                    .plugged
                    .saturating_sub(self.step)
                    .max(self.requested)
                    .max(floor);
            }
            Ok(self.level_now())
        }

        fn set_memory_max(&mut self, bytes: u64) -> Result<(), ()> {
            self.order.push("limit");
            if self.max_fails {
                return Err(());
            }
            self.max_writes.push(bytes);
            Ok(())
        }
    }

    const FAST: Timing = Timing {
        interval: Duration::from_millis(50),
        grow: Duration::from_millis(500),
        shrink: Duration::from_millis(1500),
    };

    fn run(fake: &mut Fake, booted: u64, wanted: u64) -> MemoryResize {
        resize(fake, booted, wanted, &FAST, &mut |_| {})
    }

    #[test]
    fn a_grow_raises_the_limit_first_then_asks_then_reports_the_plugged_amount() {
        let mut fake = Fake::new(1024, 0);
        let answer = run(&mut fake, 512, 768);
        assert_eq!(answer.state, DimensionState::AppliedLive);
        assert_eq!((answer.plugged_mb, answer.total_mb), (256, 768));
        assert_eq!(
            fake.order,
            vec!["limit", "request"],
            "the limit goes up before the plug"
        );
        assert_eq!(fake.max_writes, vec![768 * MIB + HOST_OVERHEAD_BYTES]);
    }

    #[test]
    fn a_guest_that_plugs_less_than_asked_is_reported_as_what_it_plugged() {
        let mut fake = Fake::new(1024, 0);
        fake.step = 0;
        let answer = run(&mut fake, 512, 768);
        assert_eq!(answer.state, DimensionState::Partial);
        assert_eq!(answer.plugged_mb, 0, "never the requested number");
        assert_eq!(answer.reason.as_deref(), Some("guest_did_not_plug"));
    }

    #[test]
    fn a_shrink_lowers_the_limit_only_after_the_guest_gave_the_memory_back() {
        let mut fake = Fake::new(1024, 512);
        let answer = run(&mut fake, 512, 768);
        assert_eq!(answer.state, DimensionState::AppliedLive);
        assert_eq!(answer.plugged_mb, 256);
        assert_eq!(
            fake.order,
            vec!["request", "limit"],
            "the limit comes down after"
        );
        assert_eq!(
            fake.max_writes,
            vec![(512 + 256) * MIB + HOST_OVERHEAD_BYTES]
        );
    }

    #[test]
    fn a_guest_that_cannot_give_memory_back_keeps_its_limit_above_what_it_holds() {
        let mut fake = Fake::new(1024, 512);
        fake.stuck_above = Some(384 * MIB);
        let answer = run(&mut fake, 512, 640);
        assert_eq!(answer.state, DimensionState::Partial);
        assert_eq!(answer.plugged_mb, 384);
        assert_eq!(answer.reason.as_deref(), Some("guest_could_not_offline"));
        assert_eq!(
            fake.max_writes,
            vec![(512 + 384) * MIB + HOST_OVERHEAD_BYTES],
            "the limit follows what is really plugged, not what was asked"
        );
    }

    #[test]
    fn the_same_total_again_changes_nothing_and_touches_nothing() {
        let mut fake = Fake::new(1024, 256);
        let answer = run(&mut fake, 512, 768);
        assert_eq!(answer.state, DimensionState::Unchanged);
        assert!(fake.order.is_empty());
    }

    #[test]
    fn totals_outside_the_machine_are_refused_before_anything_is_written() {
        let mut fake = Fake::new(1024, 0);
        let below = run(&mut fake, 512, 256);
        assert_eq!(below.state, DimensionState::Unsupported);
        assert_eq!(below.reason.as_deref(), Some("below_booted"));
        let above = run(&mut fake, 512, 4096);
        assert_eq!(above.reason.as_deref(), Some("above_ceiling"));
        assert_eq!(above.ceiling_mb, 1536);
        assert!(fake.order.is_empty());
    }

    #[test]
    fn a_machine_without_the_device_says_so_and_keeps_the_answer() {
        let mut fake = Fake::new(1024, 0);
        fake.refuse = Some(MemoryError::NoDevice);
        let answer = run(&mut fake, 512, 768);
        assert_eq!(answer.state, DimensionState::Unsupported);
        assert_eq!(
            answer.reason.as_deref(),
            Some("generation_without_virtio_mem")
        );
    }

    #[test]
    fn a_checkpoint_in_flight_fails_the_resize_with_that_reason() {
        let mut fake = Fake::new(1024, 0);
        // The level read works; the request is what is refused.
        let mut link = BusyOnRequest(&mut fake);
        let answer = resize(&mut link, 512, 768, &FAST, &mut |_| {});
        assert_eq!(answer.state, DimensionState::Failed);
        assert_eq!(answer.reason.as_deref(), Some("checkpoint_in_flight"));
    }

    struct BusyOnRequest<'a>(&'a mut Fake);

    impl MemoryLink for BusyOnRequest<'_> {
        fn set_requested(&mut self, _bytes: u64) -> Result<MemoryLevel, LinkError> {
            Err(LinkError::Device(MemoryError::Busy))
        }
        fn level(&mut self) -> Result<MemoryLevel, LinkError> {
            self.0.level()
        }
        fn set_memory_max(&mut self, bytes: u64) -> Result<(), ()> {
            self.0.set_memory_max(bytes)
        }
    }

    #[test]
    fn a_limit_that_cannot_be_raised_stops_the_grow_before_any_memory_is_asked_for() {
        let mut fake = Fake::new(1024, 0);
        fake.max_fails = true;
        let answer = run(&mut fake, 512, 768);
        assert_eq!(answer.state, DimensionState::Failed);
        assert_eq!(answer.reason.as_deref(), Some("cgroup_write_failed"));
        assert!(!fake.order.contains(&"request"));
    }
}
