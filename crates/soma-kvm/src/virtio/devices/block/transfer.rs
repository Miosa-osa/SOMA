//! What a block device tells its driver about the largest request it answers.
//!
//! A device that says nothing leaves the driver on its own defaults, which is how a driver ends
//! up forming a request the parser has to reject. A device that declares the limit bounds the
//! driver instead. Which of the two a device is belongs to the machine contract rather than to
//! the role, because the contract is what a snapshot's compatibility is pinned to.

/// Feature: `size_max` in configuration space is valid.
pub const VIRTIO_BLK_F_SIZE_MAX: u64 = 1 << 1;
/// Feature: `seg_max` in configuration space is valid.
pub const VIRTIO_BLK_F_SEG_MAX: u64 = 1 << 2;
/// Largest number of data segments one request may use, as advertised through `seg_max`.
///
/// One, so that `seg_max * size_max` is exactly [`MAX_REQUEST_BYTES`](super::request::MAX_REQUEST_BYTES)
/// and a driver that respects both fields cannot form a request the parser rejects. It is also
/// the only shape this path has ever run: a driver that has been told nothing builds exactly it.
pub const TRANSFER_SEG_MAX: u32 = 1;

/// Whether a device tells its driver the largest request it answers.
///
/// The machine contract chooses, not the role. Machine contract version 1 has offered this
/// device since it was certified and its digest pins the allowlist, so version 1 keeps saying
/// nothing and only the version 2 shape declares.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferShape {
    /// Offer neither `size_max` nor `seg_max`; the driver keeps its own defaults.
    Undeclared,
    /// Offer both, carrying the parser's own limit and [`TRANSFER_SEG_MAX`].
    Declared,
}

impl TransferShape {
    /// The feature bits that make the two configuration fields readable.
    pub(super) const fn features(self) -> u64 {
        match self {
            Self::Undeclared => 0,
            Self::Declared => VIRTIO_BLK_F_SIZE_MAX | VIRTIO_BLK_F_SEG_MAX,
        }
    }
}
