//! What each Generation's guest read from its head, kept for the next head to be warmed with.
//!
//! A head is a fresh inode whose pages are shared with the Generation's template but not resident
//! in it, so a guest's first read of one faults every page it touches in from the device. Which
//! ranges those were is a property of the Generation, so one set is kept per primed key and
//! replayed when the next head for that Generation is built.
//!
//! The set belongs to the pool rather than to any host: the machine that recorded it is gone by
//! the time the next one is prepared.

use crate::backend::kvm::prepared::PreparedGeneration;
use soma_kvm::head_reads::{HeadReads, HeadSpan};

use super::{PrimedKey, locked, pool};

/// The pool key one Generation's head belongs to, as a claim builds it.
pub(crate) fn key_for(prepared: &PreparedGeneration) -> PrimedKey {
    PrimedKey {
        reference: prepared.reference.clone(),
        generation_id: prepared.id.clone(),
        memory_mib: prepared.manifest.shape.memory_bytes / (1024 * 1024),
    }
}

/// Records what a guest read from its head, for the next head of the same Generation to be warmed
/// with.
///
/// Absorbing rather than replacing is what keeps a later or larger observation from growing the
/// set past the recorder's own bounds.
pub(crate) fn note_head_reads(key: &PrimedKey, spans: &[HeadSpan]) {
    if spans.is_empty() {
        return;
    }
    let mut warm = locked(&pool().warm);
    if let Some((_, reads)) = warm.iter_mut().find(|(held, _)| held == key) {
        reads.absorb(spans);
        return;
    }
    let mut reads = HeadReads::default();
    reads.absorb(spans);
    warm.push((key.clone(), reads));
}

/// What this Generation's guest read from its head, for a host about to build one.
///
/// Empty for a Generation nobody has launched yet, which is what keeps a Generation's first
/// launch in an API process as slow as it has always been.
pub(crate) fn warm_for(key: &PrimedKey) -> Vec<HeadSpan> {
    let mut warm = locked(&pool().warm);
    warm.iter_mut()
        .find(|(held, _)| held == key)
        .map(|(_, reads)| reads.spans().to_vec())
        .unwrap_or_default()
}
