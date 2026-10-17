//! The launch page as the agent walks it, and the uncached walks over it.
//!
//! The page is reached through `/dev/mem`, and an x86 mapping of that kind is uncached: an access
//! costs about the same whether it moves one byte or eight. Every walk here therefore moves a
//! whole word at a time, and the alignment that needs is a property of [`Page`] rather than of
//! whatever the caller happened to pass.

use std::ptr;

use soma_guest::LAUNCH_PAGE_SIZE;
use zeroize::Zeroize;

/// Exact domain bytes at the start of every valid launch page.
pub const PAGE_DOMAIN: &[u8; 16] = b"SOMA-LAUNCH-PAGE";

/// The widest access this walks the page with.
type Word = u64;
/// Bytes one access covers, and how many of them the page holds.
const WORD_BYTES: usize = size_of::<Word>();
const WORDS: usize = LAUNCH_PAGE_SIZE / WORD_BYTES;

/// One launch page as the agent walks it: a whole number of words, aligned for one.
#[repr(C, align(4096))]
pub struct Page([u8; LAUNCH_PAGE_SIZE]);

impl Page {
    /// A page of zeroes, which is what an erased page is.
    #[must_use]
    pub const fn zeroed() -> Self {
        Self([0; LAUNCH_PAGE_SIZE])
    }

    /// A page holding `byte` everywhere, for the tests that only care that it is not the domain.
    #[cfg(test)]
    pub fn filled(byte: u8) -> Self {
        Self([byte; LAUNCH_PAGE_SIZE])
    }

    /// The same bytes as a plain slice, for the locked copy the decoder reads.
    pub fn bytes_mut(&mut self) -> &mut [u8; LAUNCH_PAGE_SIZE] {
        &mut self.0
    }

    /// Whether this page still starts with the launch-page domain.
    #[must_use]
    pub fn has_domain(&self) -> bool {
        let source = self.words();
        let mut domain = [0_u8; PAGE_DOMAIN.len()];
        for (index, chunk) in domain
            .as_chunks_mut::<WORD_BYTES>()
            .0
            .iter_mut()
            .enumerate()
        {
            // SAFETY: the domain is fewer words than the page holds, so `index` stays inside
            // the borrowed view and names one whole word.
            let word = unsafe { ptr::read_volatile(source.add(index)) };
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
        domain == *PAGE_DOMAIN
    }

    /// Copies the whole page into ordinary locked memory, one uncached word at a time.
    ///
    /// Only the source is accessed a word at a time: it is the uncached side, where an access
    /// costs about the same whatever its width, so a byte-wise walk pays eight times the
    /// transactions for the same bytes. The destination is locked anonymous memory, where a byte
    /// store is as cheap as any other, so its bytes are written straight out of a register
    /// already holding the whole word.
    pub fn copy_into(&self, destination: &mut [u8; LAUNCH_PAGE_SIZE]) {
        let base = self.words();
        for index in 0..WORDS {
            // SAFETY: `Page` is aligned for `Word` and holds exactly `WORDS` of them, so `index`
            // stays inside the borrowed view and names one whole word.
            let word = unsafe { ptr::read_volatile(base.add(index)) };
            let start = index * WORD_BYTES;
            destination[start..start + WORD_BYTES].copy_from_slice(&word.to_ne_bytes());
        }
    }

    /// Overwrites the page with zeroes through volatile stores and re-reads every word.
    #[must_use]
    pub fn erase_and_verify(&mut self) -> bool {
        let base = self.words_mut();
        for index in 0..WORDS {
            // SAFETY: `Page` is aligned for `Word` and holds exactly `WORDS` of them, so `index`
            // stays inside the exclusively borrowed view and names one whole word.
            unsafe {
                ptr::write_volatile(base.add(index), 0);
                if ptr::read_volatile(base.add(index)) != 0 {
                    return false;
                }
            }
        }
        true
    }

    fn words(&self) -> *const Word {
        self.0.as_ptr().cast()
    }

    fn words_mut(&mut self) -> *mut Word {
        self.0.as_mut_ptr().cast()
    }
}

impl Zeroize for Page {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}
