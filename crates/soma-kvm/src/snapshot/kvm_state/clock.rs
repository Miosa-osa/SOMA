//! KVM clock data and the optional i8254 PIT state.

use super::{KvmStateError, invalid};
use crate::snapshot::wire::{Reader, Writer};

/// `KVM_CLOCK_TSC_STABLE | KVM_CLOCK_REALTIME | KVM_CLOCK_HOST_TSC`.
const KNOWN_CLOCK_FLAGS: u32 = 2 | 4 | 8;

/// `KVM_GET_CLOCK` result with its flags word; `realtime` and `host_tsc` are meaningful
/// only when the matching flag is set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClockState {
    pub clock: u64,
    pub flags: u32,
    pub realtime: u64,
    pub host_tsc: u64,
}

impl ClockState {
    pub const ENCODED_LEN: usize = 8 + 4 + 8 + 8;

    /// # Errors
    ///
    /// Returns [`KvmStateError::InvalidField`] when an unknown flag bit is set.
    pub fn validate(&self) -> Result<(), KvmStateError> {
        if self.flags & !KNOWN_CLOCK_FLAGS != 0 {
            return Err(invalid("clock.flags", self.flags));
        }
        Ok(())
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(Self::ENCODED_LEN);
        writer.put_u64(self.clock);
        writer.put_u32(self.flags);
        writer.put_u64(self.realtime);
        writer.put_u64(self.host_tsc);
        writer.finish()
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError`] for short, trailing, or unknown-flag input.
    pub fn decode(bytes: &[u8]) -> Result<Self, KvmStateError> {
        let mut reader = Reader::new(bytes);
        let state = Self {
            clock: reader.u64()?,
            flags: reader.u32()?,
            realtime: reader.u64()?,
            host_tsc: reader.u64()?,
        };
        reader.finish()?;
        state.validate()?;
        Ok(state)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PitChannel {
    pub count: u32,
    pub latched_count: u16,
    pub count_latched: u8,
    pub status_latched: u8,
    pub status: u8,
    pub read_state: u8,
    pub write_state: u8,
    pub write_latch: u8,
    pub rw_mode: u8,
    pub mode: u8,
    pub bcd: u8,
    pub gate: u8,
    pub count_load_time: i64,
}

impl PitChannel {
    const fn bytes(&self) -> [u8; 10] {
        [
            self.count_latched,
            self.status_latched,
            self.status,
            self.read_state,
            self.write_state,
            self.write_latch,
            self.rw_mode,
            self.mode,
            self.bcd,
            self.gate,
        ]
    }

    fn write(&self, writer: &mut Writer) {
        writer.put_u32(self.count);
        writer.put_u16(self.latched_count);
        writer.put_bytes(&self.bytes());
        writer.put_i64(self.count_load_time);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let count = reader.u32()?;
        let latched_count = reader.u16()?;
        let [
            count_latched,
            status_latched,
            status,
            read_state,
            write_state,
            write_latch,
            rw_mode,
            mode,
            bcd,
            gate,
        ] = reader.array()?;
        Ok(Self {
            count,
            latched_count,
            count_latched,
            status_latched,
            status,
            read_state,
            write_state,
            write_latch,
            rw_mode,
            mode,
            bcd,
            gate,
            count_load_time: reader.i64()?,
        })
    }
}

/// Optional `Pit` section payload (`KVM_GET_PIT2`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PitState {
    pub channels: [PitChannel; 3],
    pub flags: u32,
}

impl PitState {
    pub const ENCODED_LEN: usize = 3 * 24 + 4;

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(Self::ENCODED_LEN);
        for channel in &self.channels {
            channel.write(&mut writer);
        }
        writer.put_u32(self.flags);
        writer.finish()
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError::Wire`] for short or trailing input.
    pub fn decode(bytes: &[u8]) -> Result<Self, KvmStateError> {
        let mut reader = Reader::new(bytes);
        let mut channels = [PitChannel::default(); 3];
        for channel in &mut channels {
            *channel = PitChannel::read(&mut reader)?;
        }
        let flags = reader.u32()?;
        reader.finish()?;
        Ok(Self { channels, flags })
    }
}
