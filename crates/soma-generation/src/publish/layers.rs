use flate2::read::MultiGzDecoder;
use soma::OciDigest;

use super::ImportOciLayout;
use crate::{
    ImportError, ImportErrorKind, ImportPhase,
    layer_tar::{self, ValidatedLayerTar},
    layout::Layout,
    manifest::LayerRecord,
    oci::{Descriptor, GZIP_LAYER},
    store::{StagedObject, Store},
    verify::SelectedImage,
};

pub(super) fn publish_layers(
    layout: &Layout,
    store: &Store,
    selected: &SelectedImage,
    request: ImportOciLayout<'_>,
) -> Result<Vec<LayerRecord>, ImportError> {
    let mut pending = Vec::with_capacity(selected.layers.len());
    for (descriptor, expected_diff_id) in &selected.layers {
        let mut source = layout.open_blob(
            descriptor,
            request.limits.max_blob_bytes,
            ImportPhase::VerifyLayer,
        )?;
        let staged = store.stage_descriptor(
            &mut source,
            descriptor,
            request.limits.max_blob_bytes,
            ImportPhase::VerifyLayer,
        )?;
        pending.push(PendingLayer {
            staged,
            descriptor: descriptor.clone(),
            expected_diff_id: expected_diff_id.clone(),
        });
    }

    let records = verify_all(&mut pending, request.limits.max_expanded_bytes)?;
    for layer in pending {
        layer.staged.publish()?;
    }
    Ok(records)
}

fn verify_all(
    pending: &mut [PendingLayer<'_>],
    maximum: u64,
) -> Result<Vec<LayerRecord>, ImportError> {
    let mut expanded_total = 0_u64;
    let mut records = Vec::with_capacity(pending.len());
    for layer in pending {
        let remaining = maximum
            .checked_sub(expanded_total)
            .ok_or_else(layer_limit)?;
        let validated =
            validate_layer_tar(&mut layer.staged, &layer.descriptor.media_type, remaining)?;
        if validated.diff_id != layer.expected_diff_id {
            return Err(ImportError::new(
                ImportPhase::VerifyLayer,
                ImportErrorKind::Integrity,
            ));
        }
        expanded_total = expanded_total
            .checked_add(validated.expanded_size)
            .ok_or_else(layer_limit)?;
        records.push(LayerRecord {
            descriptor: layer.descriptor.clone(),
            diff_id: validated.diff_id,
            expanded_size: validated.expanded_size,
            entry_count: validated.entry_count,
        });
    }
    Ok(records)
}

fn validate_layer_tar(
    staged: &mut StagedObject<'_>,
    media_type: &str,
    maximum: u64,
) -> Result<ValidatedLayerTar, ImportError> {
    let file = staged.reader()?;
    if media_type == GZIP_LAYER {
        layer_tar::validate(&mut MultiGzDecoder::new(file), maximum)
    } else {
        layer_tar::validate(file, maximum)
    }
}

const fn layer_limit() -> ImportError {
    ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::LimitExceeded)
}

struct PendingLayer<'store> {
    staged: StagedObject<'store>,
    descriptor: Descriptor,
    expected_diff_id: OciDigest,
}
