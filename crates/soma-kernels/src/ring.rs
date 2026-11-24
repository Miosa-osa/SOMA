//! `VirtIO` ring positions.

/// A ring size that is a nonzero power of two: the only size a `VirtIO` queue may activate.
///
/// The property is checked once, when the size is accepted, so naming a slot costs one AND
/// instead of the division `index % size` compiles to: the compiler cannot see that a size read
/// at run time is a power of two, but this type can only hold one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RingSize(u16);

impl RingSize {
    /// The ring size `size`, or `None` when it is zero or not a power of two.
    #[inline]
    #[must_use]
    pub const fn new(size: u16) -> Option<Self> {
        if size.is_power_of_two() {
            Some(Self(size))
        } else {
            None
        }
    }

    /// The number of entries.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The slot a free-running 16-bit ring index names: exactly `index % self.get()`.
    #[inline]
    #[must_use]
    pub const fn slot(self, index: u16) -> u16 {
        index & (self.0 - 1)
    }
}

/// The reference: the remainder the queue computed before [`RingSize::slot`].
///
/// # Panics
/// Panics when `size` is zero.
#[inline]
#[must_use]
pub const fn ring_slot_reference(index: u16, size: u16) -> u16 {
    index % size
}
