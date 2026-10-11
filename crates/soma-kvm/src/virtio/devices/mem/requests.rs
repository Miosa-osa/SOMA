//! The five requests a guest driver sends, each checked against the region and against what the
//! host asked for before it changes anything.

use super::{
    BLOCK_SIZE, MAX_REQUEST_BLOCKS, MemDevice, REQ_PLUG, REQ_STATE, REQ_UNPLUG, REQ_UNPLUG_ALL,
    REQUEST_LEN, RESP_ACK, RESP_ERROR, RESP_NACK, STATE_MIXED, STATE_PLUGGED, STATE_UNPLUGGED,
};

impl MemDevice {
    pub(super) fn block_of(&self, addr: u64) -> Option<u64> {
        let offset = addr.checked_sub(self.addr)?;
        (offset.is_multiple_of(BLOCK_SIZE) && offset < self.region_size)
            .then_some(offset / BLOCK_SIZE)
    }

    fn bit(&self, block: u64) -> bool {
        self.bitmap[usize::try_from(block / 64).unwrap_or(usize::MAX)] >> (block % 64) & 1 == 1
    }

    fn set_bit(&mut self, block: u64, on: bool) {
        let word = &mut self.bitmap[usize::try_from(block / 64).unwrap_or(usize::MAX)];
        if on {
            *word |= 1 << (block % 64);
        } else {
            *word &= !(1 << (block % 64));
        }
    }

    /// The first block of `[addr, addr + count blocks)` and the count, when the range is valid.
    fn range(&self, addr: u64, count: u64) -> Option<(u64, u64)> {
        let first = self.block_of(addr)?;
        let end = first.checked_add(count)?;
        (count > 0 && count <= MAX_REQUEST_BLOCKS && end <= self.region_size / BLOCK_SIZE)
            .then_some((first, count))
    }

    pub(super) fn answer(&mut self, request: &[u8; REQUEST_LEN]) -> (u16, u16) {
        let kind = u16::from_le_bytes([request[0], request[1]]);
        let addr = u64::from_le_bytes(request[8..16].try_into().unwrap_or([0; 8]));
        let count = u64::from(u16::from_le_bytes([request[16], request[17]]));
        match kind {
            REQ_PLUG => self.plug(addr, count),
            REQ_UNPLUG => self.unplug(addr, count),
            REQ_UNPLUG_ALL => self.unplug_all(),
            REQ_STATE => self.state(addr, count),
            _ => self.error(),
        }
    }

    const fn error(&mut self) -> (u16, u16) {
        self.counters.errored = self.counters.errored.saturating_add(1);
        (RESP_ERROR, 0)
    }

    fn plug(&mut self, addr: u64, count: u64) -> (u16, u16) {
        let Some((first, count)) = self.range(addr, count) else {
            return self.error();
        };
        if (first..first + count).any(|block| self.bit(block)) {
            return self.error();
        }
        // The guest may hold no more than the host asked for.
        if (self.plugged_blocks + count) * BLOCK_SIZE > self.requested {
            self.counters.nacked = self.counters.nacked.saturating_add(1);
            return (RESP_NACK, 0);
        }
        if self
            .backing
            .plug(first * BLOCK_SIZE, count * BLOCK_SIZE)
            .is_err()
        {
            return self.error();
        }
        for block in first..first + count {
            self.set_bit(block, true);
        }
        self.plugged_blocks += count;
        self.counters.plugged = self.counters.plugged.saturating_add(1);
        (RESP_ACK, 0)
    }

    fn unplug(&mut self, addr: u64, count: u64) -> (u16, u16) {
        let Some((first, count)) = self.range(addr, count) else {
            return self.error();
        };
        if !(first..first + count).all(|block| self.bit(block)) {
            return self.error();
        }
        if self
            .backing
            .discard(first * BLOCK_SIZE, count * BLOCK_SIZE)
            .is_err()
        {
            return self.error();
        }
        for block in first..first + count {
            self.set_bit(block, false);
        }
        self.plugged_blocks -= count;
        self.counters.unplugged = self.counters.unplugged.saturating_add(1);
        (RESP_ACK, 0)
    }

    fn unplug_all(&mut self) -> (u16, u16) {
        if self.backing.discard(0, self.region_size).is_err() {
            return self.error();
        }
        self.bitmap.fill(0);
        self.plugged_blocks = 0;
        self.counters.unplugged = self.counters.unplugged.saturating_add(1);
        (RESP_ACK, 0)
    }

    fn state(&mut self, addr: u64, count: u64) -> (u16, u16) {
        let Some((first, count)) = self.range(addr, count) else {
            return self.error();
        };
        let plugged = (first..first + count)
            .filter(|block| self.bit(*block))
            .count() as u64;
        let state = match plugged {
            0 => STATE_UNPLUGGED,
            n if n == count => STATE_PLUGGED,
            _ => STATE_MIXED,
        };
        (RESP_ACK, state)
    }
}
