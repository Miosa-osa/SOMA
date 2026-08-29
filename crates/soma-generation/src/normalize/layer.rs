use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek as _, SeekFrom},
};

use flate2::read::MultiGzDecoder;

use super::{
    entry::{self, GuestPath, PlannedNode},
    node_plan, pax,
    stream::{ExpandedStream, StreamState},
};
use crate::{
    ImportPhase, NormalizeError, NormalizeErrorKind, NormalizePhase, RootfsLimits,
    manifest::LayerRecord,
    oci::GZIP_LAYER,
    store::Store,
    tar_preflight::{ExtensionPolicy, MAX_LOCAL_PAX_RECORD_BYTES, PreflightBudget, PreflightError},
};

#[cfg(test)]
mod tests;

pub(super) enum Whiteout {
    Remove(GuestPath),
    Opaque(GuestPath),
}

pub(super) struct LayerPlan {
    pub(super) additions: BTreeMap<GuestPath, PlannedNode>,
    pub(super) whiteouts: Vec<Whiteout>,
}

pub(super) struct Budget {
    entries: u32,
    metadata_bytes: u64,
    content_bytes: u64,
}

impl Budget {
    pub(super) const fn new() -> Self {
        Self {
            entries: 0,
            metadata_bytes: 0,
            content_bytes: 0,
        }
    }

    fn observe(
        &mut self,
        path: &[u8],
        link: Option<&[u8]>,
        size: u64,
        limits: RootfsLimits,
    ) -> Result<(), NormalizeError> {
        self.entries = self.entries.checked_add(1).ok_or_else(limit)?;
        let metadata = path
            .len()
            .checked_add(link.map_or(0, <[u8]>::len))
            .ok_or_else(limit)?;
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(u64::try_from(metadata).map_err(|_| limit())?)
            .ok_or_else(limit)?;
        self.content_bytes = self.content_bytes.checked_add(size).ok_or_else(limit)?;
        if self.entries > limits.max_entries
            || self.metadata_bytes > limits.max_metadata_bytes
            || self.content_bytes > limits.max_content_bytes
        {
            return Err(limit());
        }
        Ok(())
    }
}

pub(super) fn parse(
    store: &Store,
    record: &LayerRecord,
    limits: RootfsLimits,
    budget: &mut Budget,
    preflight_budget: &mut PreflightBudget,
) -> Result<LayerPlan, NormalizeError> {
    let mut file = store
        .open_verified_blob(
            &record.descriptor,
            limits.max_blob_bytes,
            ImportPhase::VerifyLayer,
        )
        .map_err(|error| NormalizeError::from_import(NormalizePhase::VerifyLayer, error))?;
    let policy = ExtensionPolicy {
        long_record_ceiling: u64::from(limits.max_path_bytes) + 1,
        pax_record_ceiling: MAX_LOCAL_PAX_RECORD_BYTES,
    };
    if record.descriptor.media_type == GZIP_LAYER {
        crate::tar_preflight::preflight(
            MultiGzDecoder::new(&mut file),
            record.expanded_size,
            policy,
            preflight_budget,
        )
        .map_err(map_preflight_error)?;
        file.seek(SeekFrom::Start(0)).map_err(|_| io_error())?;
        parse_reader(MultiGzDecoder::new(file), store, record, limits, budget)
    } else {
        crate::tar_preflight::preflight(&mut file, record.expanded_size, policy, preflight_budget)
            .map_err(map_preflight_error)?;
        file.seek(SeekFrom::Start(0)).map_err(|_| io_error())?;
        parse_reader(file, store, record, limits, budget)
    }
}

fn parse_reader<R: Read>(
    reader: R,
    store: &Store,
    record: &LayerRecord,
    limits: RootfsLimits,
    budget: &mut Budget,
) -> Result<LayerPlan, NormalizeError> {
    let state = StreamState::default();
    let maximum = record.expanded_size.checked_add(1).ok_or_else(limit)?;
    let mut stream = ExpandedStream::new(reader, maximum, &state);
    let mut plan = LayerPlan {
        additions: BTreeMap::new(),
        whiteouts: Vec::new(),
    };
    let observed = {
        let mut archive = tar::Archive::new(&mut stream);
        let entries = archive.entries().map_err(|_| state.error())?;
        let mut seen = BTreeSet::new();
        let mut count = 0_u32;
        for entry in entries {
            let mut entry = entry.map_err(|_| state.error())?;
            count = count.checked_add(1).ok_or_else(limit)?;
            parse_entry(&mut entry, store, limits, budget, &mut seen, &mut plan)
                .map_err(|error| prefer_stream_error(error, &state))?;
        }
        count
    };
    stream.validate_tail()?;
    let (diff_id, expanded_size) = stream.finish();
    if diff_id != record.diff_id
        || expanded_size != record.expanded_size
        || observed != record.entry_count
    {
        return Err(integrity());
    }
    Ok(plan)
}

fn parse_entry<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    store: &Store,
    limits: RootfsLimits,
    budget: &mut Budget,
    seen: &mut BTreeSet<GuestPath>,
    plan: &mut LayerPlan,
) -> Result<(), NormalizeError> {
    let names = pax::effective_names(entry)?;
    let path = entry::normalize_path(&names.path, limits.max_path_bytes)?;
    if !seen.insert(path.clone()) {
        return Err(invalid());
    }
    let kind = entry.header().entry_type();
    let size = entry.size();
    if entry::parent(&path)
        .split(|byte| *byte == b'/')
        .any(|component| component.starts_with(b".wh."))
    {
        return Err(invalid());
    }
    if entry::basename(&path).starts_with(b".wh.") {
        if !kind.is_file() || names.link.is_some() {
            return Err(invalid());
        }
        budget.observe(&path, None, size, limits)?;
        return parse_whiteout(&path, size, plan);
    }
    budget.observe(&path, names.link.as_deref(), size, limits)?;
    let node = node_plan::parse(entry, store, limits, kind, size, names.link.as_deref())?;
    if path.is_empty() && !matches!(node, PlannedNode::Directory(_)) {
        return Err(invalid());
    }
    plan.additions.insert(path, node);
    Ok(())
}

fn parse_whiteout(path: &[u8], size: u64, plan: &mut LayerPlan) -> Result<(), NormalizeError> {
    if size != 0 || path.is_empty() {
        return Err(invalid());
    }
    let parent = entry::parent(path);
    let name = entry::basename(path);
    if name == b".wh..wh..opq" {
        plan.whiteouts.push(Whiteout::Opaque(parent.to_vec()));
        return Ok(());
    }
    let target = name.strip_prefix(b".wh.").ok_or_else(invalid)?;
    if target.is_empty() || matches!(target, b"." | b"..") {
        return Err(invalid());
    }
    plan.whiteouts
        .push(Whiteout::Remove(entry::child(parent, target)));
    Ok(())
}

fn prefer_stream_error(error: NormalizeError, state: &StreamState) -> NormalizeError {
    state.failure().unwrap_or(error)
}

const fn invalid() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::InvalidInput)
}

const fn integrity() -> NormalizeError {
    NormalizeError::new(NormalizePhase::VerifyLayer, NormalizeErrorKind::Integrity)
}

const fn limit() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::ApplyLayer,
        NormalizeErrorKind::LimitExceeded,
    )
}

const fn io_error() -> NormalizeError {
    NormalizeError::new(NormalizePhase::VerifyLayer, NormalizeErrorKind::Io)
}

const fn map_preflight_error(error: PreflightError) -> NormalizeError {
    let kind = match error {
        PreflightError::Unsupported => NormalizeErrorKind::Unsupported,
        PreflightError::LimitExceeded => NormalizeErrorKind::LimitExceeded,
        PreflightError::Integrity => NormalizeErrorKind::Integrity,
        PreflightError::Io => NormalizeErrorKind::Io,
    };
    NormalizeError::new(NormalizePhase::VerifyLayer, kind)
}
