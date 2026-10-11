//! The plugged-block bitmap: one bit per device block, bounded at construction.
//!
//! Every range operation validates `first + count` with checked arithmetic
//! against the block count before touching a word, so a hostile request can
//! neither index past the bitmap nor overflow. Bits at or beyond the block
//! count are always zero, which keeps the plugged count equal to the popcount.

use std::fmt;

/// Largest number of blocks one device may track (a 128 KiB bitmap).
pub const MAX_BLOCKS: usize = 1 << 20;

/// A rejected bitmap operation or encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BitmapError {
    /// The block count is zero or above [`MAX_BLOCKS`].
    BadBlockCount,
    /// The range is empty, overflows, or leaves the tracked blocks.
    BadRange,
    /// A plug touched an already plugged block.
    AlreadyPlugged,
    /// An unplug touched a block that is not plugged.
    NotPlugged,
    /// A decoded bitmap has the wrong length or bits past the block count.
    Malformed,
}

impl fmt::Display for BitmapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "plugged bitmap rejected: {self:?}")
    }
}

impl std::error::Error for BitmapError {}

/// Plug state of a block range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeState {
    /// Every block in the range is plugged.
    Plugged,
    /// No block in the range is plugged.
    Unplugged,
    /// Both plugged and unplugged blocks are present.
    Mixed,
}

/// One bit per block; set means plugged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluggedBitmap {
    words: Vec<u64>,
    blocks: usize,
    plugged: usize,
}

impl PluggedBitmap {
    /// An all-unplugged bitmap over `blocks` blocks.
    ///
    /// # Errors
    /// Rejects a zero or oversized block count.
    pub fn new(blocks: usize) -> Result<Self, BitmapError> {
        if blocks == 0 || blocks > MAX_BLOCKS {
            return Err(BitmapError::BadBlockCount);
        }
        Ok(Self {
            words: vec![0; blocks.div_ceil(64)],
            blocks,
            plugged: 0,
        })
    }

    /// Number of tracked blocks.
    #[must_use]
    pub const fn blocks(&self) -> usize {
        self.blocks
    }

    /// Number of plugged blocks.
    #[must_use]
    pub const fn plugged_blocks(&self) -> usize {
        self.plugged
    }

    /// Whether `block` is plugged; out-of-range blocks are not.
    #[must_use]
    pub fn is_plugged(&self, block: usize) -> bool {
        block < self.blocks && (self.words[block / 64] >> (block % 64)) & 1 == 1
    }

    /// Plug state of `[first, first + count)`.
    ///
    /// # Errors
    /// Rejects an empty, overflowing, or out-of-range request.
    pub fn range_state(&self, first: usize, count: usize) -> Result<RangeState, BitmapError> {
        self.check(first, count)?;
        let set = self.set_in(first, count);
        Ok(if set == 0 {
            RangeState::Unplugged
        } else if set == count {
            RangeState::Plugged
        } else {
            RangeState::Mixed
        })
    }

    /// Plugs `[first, first + count)`; every block must be unplugged.
    ///
    /// # Errors
    /// Rejects a bad range or any already plugged block; state is unchanged.
    pub fn plug(&mut self, first: usize, count: usize) -> Result<(), BitmapError> {
        self.check(first, count)?;
        if self.set_in(first, count) != 0 {
            return Err(BitmapError::AlreadyPlugged);
        }
        self.apply(first, count, true);
        self.plugged += count;
        Ok(())
    }

    /// Unplugs `[first, first + count)`; every block must be plugged.
    ///
    /// # Errors
    /// Rejects a bad range or any unplugged block; state is unchanged.
    pub fn unplug(&mut self, first: usize, count: usize) -> Result<(), BitmapError> {
        self.check(first, count)?;
        if self.set_in(first, count) != count {
            return Err(BitmapError::NotPlugged);
        }
        self.apply(first, count, false);
        self.plugged -= count;
        Ok(())
    }

    /// Unplugs everything.
    pub fn clear(&mut self) {
        self.words.fill(0);
        self.plugged = 0;
    }

    /// Encodes the bitmap as `ceil(blocks / 8)` little-endian bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut raw: Vec<u8> = self
            .words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        raw.resize(self.blocks.div_ceil(8), 0);
        raw
    }

    /// Decodes a bitmap over exactly `blocks` blocks.
    ///
    /// # Errors
    /// Rejects a wrong length or any set bit at or past `blocks`.
    pub fn from_bytes(blocks: usize, raw: &[u8]) -> Result<Self, BitmapError> {
        let mut bitmap = Self::new(blocks)?;
        if raw.len() != blocks.div_ceil(8) {
            return Err(BitmapError::Malformed);
        }
        for (index, chunk) in raw.chunks(8).enumerate() {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            bitmap.words[index] = u64::from_le_bytes(word);
        }
        let tail = blocks % 64;
        if tail != 0 && bitmap.words[bitmap.words.len() - 1] >> tail != 0 {
            return Err(BitmapError::Malformed);
        }
        bitmap.plugged = bitmap
            .words
            .iter()
            .map(|word| usize::try_from(word.count_ones()).unwrap_or(usize::MAX))
            .sum();
        Ok(bitmap)
    }

    fn check(&self, first: usize, count: usize) -> Result<(), BitmapError> {
        match first.checked_add(count) {
            Some(end) if count > 0 && end <= self.blocks => Ok(()),
            _ => Err(BitmapError::BadRange),
        }
    }

    /// Visits each `(word index, mask)` covering a validated range.
    fn spans(first: usize, count: usize, mut visit: impl FnMut(usize, u64)) {
        let mut index = first;
        let mut remaining = count;
        while remaining > 0 {
            let low = index % 64;
            let take = (64 - low).min(remaining);
            let mask = if take == 64 {
                u64::MAX
            } else {
                ((1u64 << take) - 1) << low
            };
            visit(index / 64, mask);
            index += take;
            remaining -= take;
        }
    }

    fn set_in(&self, first: usize, count: usize) -> usize {
        let mut set = 0usize;
        Self::spans(first, count, |word, mask| {
            set += usize::try_from((self.words[word] & mask).count_ones()).unwrap_or(0);
        });
        set
    }

    fn apply(&mut self, first: usize, count: usize, plug: bool) {
        let words = &mut self.words;
        Self::spans(first, count, |word, mask| {
            if plug {
                words[word] |= mask;
            } else {
                words[word] &= !mask;
            }
        });
    }
}
