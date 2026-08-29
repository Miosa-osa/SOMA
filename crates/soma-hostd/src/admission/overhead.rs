//! The measured host-side memory cost of one VM, with the label that says where it came from.
//!
//! A placeholder must say it is a placeholder: the capacity ladder multiplies this number by
//! every resident Instance, so an unlabelled guess would become a capacity claim.

/// The measured host-side memory cost of one VM beyond its guest memory.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MeasuredOverhead {
    /// Bytes per resident Instance.
    pub bytes_per_instance: u64,
    /// Where the number comes from; a placeholder must say so.
    pub evidence: &'static str,
}

impl MeasuredOverhead {
    /// The 64 MiB explanatory placeholder from the visual atlas; not a measurement.
    pub const ATLAS_PLACEHOLDER: Self = Self {
        bytes_per_instance: 64 << 20,
        evidence: "docs/architecture/visual-atlas.md section 15 placeholder; not measured",
    };

    /// The single-sample non-guest resident total of the `x86_64` PVH boot proof, rounded up
    /// to 4 MiB; a debug-build diagnostic, not a certified per-VM figure.
    pub const PVH_BOOT_SINGLE_SAMPLE: Self = Self {
        bytes_per_instance: 4 << 20,
        evidence: "docs/evidence/2026-08-29-x86_64-pvh-kernel-boot.md single-sample debug-build non-guest resident total of about 3.6 MiB; not certified",
    };
}
