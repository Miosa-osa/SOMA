//! Mapping a captured memory image into a machine's guest RAM, and the diagnostic that makes
//! it resident first.
//!
//! The mapping is private and never copied, so a hundred Instances of one Generation share one
//! set of page-cache pages and each pays only for the pages its own guest writes.
//!
//! Whether the guest finds that memory resident or faults it in a page at a time separates two
//! candidate explanations of where a resume spends its time, and only trying both on one host
//! tells them apart. The walk is linear in the whole image and costs hundreds of milliseconds
//! on a large one, so it is never a default: an operator sets the variable, measures, unsets
//! it. Nothing about the mapping's guarantees changes when they do, because the walk only
//! reads: every page stays shared, and a guest write still takes its own private copy.

use std::fs::File;

use super::{Artifact, GuestLayout, GuestRam, RamMapping, SnapshotError};
use crate::snapshot::{
    manifest::Manifest,
    memory::{MappingError, PrivateMapping},
};
use crate::x86_64::sandbox::{Milestone, Timeline};

/// Names the diagnostic. Never set on a host serving requests.
const PREFAULT_MEMORY: &str = "SOMA_KVM_PREFAULT_MEMORY";

/// Maps the certified memory object privately and hands the range to a fresh [`GuestRam`].
///
/// # Errors
///
/// Returns the typed failure of opening, mapping, or laying out the memory object.
pub(super) fn map(
    path: &std::path::Path,
    manifest: &Manifest,
    timeline: &mut Timeline,
) -> Result<GuestRam, SnapshotError> {
    let size = manifest.header().memory.size();
    let layout = GuestLayout::new(size)?;
    let memory =
        File::open(path).map_err(|error| SnapshotError::io(Artifact::Memory, "open", &error))?;
    let mapping = PrivateMapping::map(&memory, size)?;
    if std::env::var_os(PREFAULT_MEMORY).is_some() {
        let _ignored = mapping.prefault();
        timeline.mark(Milestone::PrefaultMemory);
    }
    let (base, len) = mapping.into_raw();
    let base =
        std::ptr::NonNull::new(base).ok_or(SnapshotError::Mapping(MappingError::ZeroLength))?;
    // The machine now owns the range and unmaps it exactly once, after the VM is released.
    let ram = GuestRam::from_mapping(RamMapping::adopt(base, len), layout)?;
    drop(memory);
    Ok(ram)
}
