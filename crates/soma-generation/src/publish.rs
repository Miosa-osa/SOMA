use std::{collections::BTreeMap, io::Cursor};

use soma::{OciDigest, OciPlatform, WorkloadIdentity};

use crate::{
    ImportError, ImportErrorKind, ImportOciLayout, ImportPhase, ImportedOci, OciSelection, digest,
    layout::Layout,
    manifest::{self},
    oci::{Descriptor, INDEX_MEDIA_TYPE},
    store::Store,
    traversal::Traversal,
    verify::SelectedImage,
};

mod layers;

use layers::publish_layers;

pub(crate) fn publish(
    layout: &Layout,
    traversal: Traversal,
    selected: SelectedImage,
    request: ImportOciLayout<'_>,
) -> Result<ImportedOci, ImportError> {
    let mut artifacts = Artifacts::new(request.limits.max_total_blob_bytes);
    let top_size = u64::try_from(traversal.top_index_bytes.len()).map_err(|_| blob_limit())?;
    require_blob_size(top_size, request.limits.max_blob_bytes)?;
    let top_descriptor = Descriptor {
        media_type: INDEX_MEDIA_TYPE.to_owned(),
        digest: digest::bytes(&traversal.top_index_bytes),
        size: top_size,
        platform: None,
    };
    artifacts.reserve(&top_descriptor, true)?;
    for index in &selected.candidate.indexes {
        artifacts.reserve(index, true)?;
    }
    let manifest = &selected.candidate.manifest;
    artifacts.reserve(manifest, true)?;
    artifacts.reserve(&selected.config, true)?;
    for (layer, _) in &selected.layers {
        artifacts.reserve(layer, true)?;
    }

    let store = Store::open(request.store)?;
    let top = store.put_bytes(
        &traversal.top_index_bytes,
        INDEX_MEDIA_TYPE,
        ImportPhase::Publish,
    )?;
    debug_assert_eq!(top.digest, top_descriptor.digest);
    publish_indexes(layout, &store, &selected, request)?;

    store.put_descriptor(
        &mut Cursor::new(&selected.manifest_bytes),
        manifest,
        request.limits.max_blob_bytes,
        ImportPhase::VerifyManifest,
    )?;
    store.put_descriptor(
        &mut Cursor::new(&selected.config_bytes),
        &selected.config,
        request.limits.max_blob_bytes,
        ImportPhase::VerifyConfig,
    )?;

    let layer_records = publish_layers(layout, &store, &selected, request)?;
    let workload = workload(
        request.selection,
        manifest.digest.clone(),
        selected.platform.clone(),
    );
    let import_bytes = manifest::encode(&workload, manifest, &selected.config, &layer_records);
    require_blob_size(
        u64::try_from(import_bytes.len()).map_err(|_| blob_limit())?,
        request.limits.max_blob_bytes,
    )?;
    let import_descriptor = Descriptor {
        media_type: manifest::IMPORT_MEDIA_TYPE.to_owned(),
        digest: digest::bytes(&import_bytes),
        size: u64::try_from(import_bytes.len()).map_err(|_| blob_limit())?,
        platform: None,
    };
    artifacts.reserve(&import_descriptor, false)?;
    store.put_descriptor(
        &mut Cursor::new(&import_bytes),
        &import_descriptor,
        request.limits.max_blob_bytes,
        ImportPhase::Publish,
    )?;

    let mut traversed_indexes = Vec::with_capacity(selected.candidate.indexes.len() + 1);
    traversed_indexes.push(traversal.top_index_digest);
    traversed_indexes.extend(
        selected
            .candidate
            .indexes
            .into_iter()
            .map(|index| index.digest),
    );
    Ok(ImportedOci {
        workload,
        import_manifest_digest: import_descriptor.digest,
        import_manifest_size: import_descriptor.size,
        stored_blob_count: artifacts.count()?,
        stored_bytes: artifacts.bytes,
        traversed_indexes,
    })
}

fn publish_indexes(
    layout: &Layout,
    store: &Store,
    selected: &SelectedImage,
    request: ImportOciLayout<'_>,
) -> Result<(), ImportError> {
    for index in &selected.candidate.indexes {
        let mut source = layout.open_blob(
            index,
            request.limits.max_blob_bytes,
            ImportPhase::SelectManifest,
        )?;
        store.put_descriptor(
            &mut source,
            index,
            request.limits.max_blob_bytes,
            ImportPhase::SelectManifest,
        )?;
    }
    Ok(())
}

fn workload(
    selection: OciSelection<'_>,
    manifest_digest: OciDigest,
    platform: OciPlatform,
) -> WorkloadIdentity {
    match selection {
        OciSelection::Exact(identity) => {
            let workload = WorkloadIdentity::new(manifest_digest, platform, None);
            match identity.index_digest() {
                Some(index) => workload.with_index_digest(index.clone()),
                None => workload,
            }
        }
        OciSelection::Platform(_) => WorkloadIdentity::new(manifest_digest, platform, None),
    }
}

fn require_blob_size(size: u64, maximum: u64) -> Result<(), ImportError> {
    if size > maximum {
        return Err(blob_limit());
    }
    Ok(())
}

const fn blob_limit() -> ImportError {
    ImportError::new(ImportPhase::Publish, ImportErrorKind::LimitExceeded)
}

struct Artifacts {
    sizes: BTreeMap<String, u64>,
    source_limit: u64,
    source_bytes: u64,
    bytes: u64,
}

impl Artifacts {
    const fn new(source_limit: u64) -> Self {
        Self {
            sizes: BTreeMap::new(),
            source_limit,
            source_bytes: 0,
            bytes: 0,
        }
    }

    fn reserve(&mut self, descriptor: &Descriptor, source: bool) -> Result<(), ImportError> {
        let key = descriptor.digest.as_str().to_owned();
        if let Some(size) = self.sizes.get(&key) {
            if *size != descriptor.size {
                return Err(ImportError::new(
                    ImportPhase::Publish,
                    ImportErrorKind::Integrity,
                ));
            }
            return Ok(());
        }
        self.bytes = self
            .bytes
            .checked_add(descriptor.size)
            .ok_or_else(blob_limit)?;
        if source {
            self.source_bytes = self
                .source_bytes
                .checked_add(descriptor.size)
                .ok_or_else(blob_limit)?;
            if self.source_bytes > self.source_limit {
                return Err(blob_limit());
            }
        }
        self.sizes.insert(key, descriptor.size);
        Ok(())
    }

    fn count(&self) -> Result<u32, ImportError> {
        u32::try_from(self.sizes.len()).map_err(|_| blob_limit())
    }
}
