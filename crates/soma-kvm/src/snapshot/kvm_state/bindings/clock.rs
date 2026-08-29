//! KVM clock and PIT conversions.

use kvm_bindings::{kvm_clock_data, kvm_pit_channel_state, kvm_pit_state2};

use super::BindingError;
use crate::snapshot::kvm_state::{ClockState, PitChannel, PitState};

impl TryFrom<kvm_clock_data> for ClockState {
    type Error = BindingError;

    fn try_from(clock: kvm_clock_data) -> Result<Self, BindingError> {
        let state = Self {
            clock: clock.clock,
            flags: clock.flags,
            realtime: clock.realtime,
            host_tsc: clock.host_tsc,
        };
        state.validate()?;
        Ok(state)
    }
}

impl From<ClockState> for kvm_clock_data {
    fn from(clock: ClockState) -> Self {
        Self {
            clock: clock.clock,
            flags: clock.flags,
            pad0: 0,
            realtime: clock.realtime,
            host_tsc: clock.host_tsc,
            pad: [0; 4],
        }
    }
}

impl From<kvm_pit_channel_state> for PitChannel {
    fn from(channel: kvm_pit_channel_state) -> Self {
        Self {
            count: channel.count,
            latched_count: channel.latched_count,
            count_latched: channel.count_latched,
            status_latched: channel.status_latched,
            status: channel.status,
            read_state: channel.read_state,
            write_state: channel.write_state,
            write_latch: channel.write_latch,
            rw_mode: channel.rw_mode,
            mode: channel.mode,
            bcd: channel.bcd,
            gate: channel.gate,
            count_load_time: channel.count_load_time,
        }
    }
}

impl From<PitChannel> for kvm_pit_channel_state {
    fn from(channel: PitChannel) -> Self {
        Self {
            count: channel.count,
            latched_count: channel.latched_count,
            count_latched: channel.count_latched,
            status_latched: channel.status_latched,
            status: channel.status,
            read_state: channel.read_state,
            write_state: channel.write_state,
            write_latch: channel.write_latch,
            rw_mode: channel.rw_mode,
            mode: channel.mode,
            bcd: channel.bcd,
            gate: channel.gate,
            count_load_time: channel.count_load_time,
        }
    }
}

impl From<kvm_pit_state2> for PitState {
    fn from(pit: kvm_pit_state2) -> Self {
        Self {
            channels: pit.channels.map(PitChannel::from),
            flags: pit.flags,
        }
    }
}

impl From<PitState> for kvm_pit_state2 {
    fn from(pit: PitState) -> Self {
        Self {
            channels: pit.channels.map(kvm_pit_channel_state::from),
            flags: pit.flags,
            reserved: [0; 9],
        }
    }
}

#[cfg(test)]
mod tests {
    use kvm_bindings::{kvm_clock_data, kvm_pit_state2};

    use crate::snapshot::kvm_state::{ClockState, PitState};

    #[test]
    fn clock_and_pit_round_trip_and_unknown_clock_flags_reject() {
        let clock = ClockState {
            clock: 10,
            flags: 2,
            realtime: 0,
            host_tsc: 0,
        };
        assert_eq!(ClockState::try_from(kvm_clock_data::from(clock)), Ok(clock));
        let bad = kvm_clock_data {
            flags: 1,
            ..kvm_clock_data::from(clock)
        };
        assert!(ClockState::try_from(bad).is_err());

        let mut pit = PitState::default();
        pit.channels[1].count = 0x1234;
        pit.channels[1].count_load_time = -5;
        assert_eq!(PitState::from(kvm_pit_state2::from(pit)), pit);
    }
}
