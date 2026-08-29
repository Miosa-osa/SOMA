use soma::{OciDigest, OciPlatform};

use crate::{
    ImportError, ImportErrorKind, ImportOciLayout, ImportPhase, OciSelection, digest,
    layout::Layout,
    oci::{
        CONFIG_MEDIA_TYPE, ConfigWire, Descriptor, GZIP_LAYER, MANIFEST_MEDIA_TYPE, ManifestWire,
        PLAIN_LAYER, merge_platform_claims, parse_json,
    },
    traversal::{Candidate, Traversal, platform_matches},
};

const MAX_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) struct SelectedImage {
    pub(crate) candidate: Candidate,
    pub(crate) manifest_bytes: Vec<u8>,
    pub(crate) config: Descriptor,
    pub(crate) config_bytes: Vec<u8>,
    pub(crate) platform: OciPlatform,
    pub(crate) layers: Vec<(Descriptor, OciDigest)>,
}

pub(crate) fn select_image(
    layout: &Layout,
    traversal: &Traversal,
    request: ImportOciLayout<'_>,
) -> Result<SelectedImage, ImportError> {
    let mut matches = Vec::new();
    let mut descriptor_count = traversal.descriptor_count;
    for candidate in &traversal.candidates {
        if let Some(image) =
            verify_candidate(layout, candidate.clone(), request, &mut descriptor_count)?
        {
            matches.push(image);
        }
    }
    match matches.len() {
        0 => Err(ImportError::new(
            ImportPhase::SelectManifest,
            ImportErrorKind::NotFound,
        )),
        1 => Ok(matches.remove(0)),
        _ => Err(ImportError::new(
            ImportPhase::SelectManifest,
            ImportErrorKind::Ambiguous,
        )),
    }
}

fn verify_candidate(
    layout: &Layout,
    candidate: Candidate,
    request: ImportOciLayout<'_>,
    descriptor_count: &mut u32,
) -> Result<Option<SelectedImage>, ImportError> {
    let document_limit = request.limits.max_blob_bytes.min(MAX_DOCUMENT_BYTES);
    let manifest_bytes = layout.read_blob(
        &candidate.manifest,
        document_limit,
        ImportPhase::VerifyManifest,
    )?;
    let manifest: ManifestWire = parse_json(&manifest_bytes, ImportPhase::VerifyManifest)?;
    if manifest.schema_version != 2
        || manifest
            .media_type
            .as_deref()
            .is_some_and(|value| value != MANIFEST_MEDIA_TYPE)
    {
        return Err(ImportError::new(
            ImportPhase::VerifyManifest,
            ImportErrorKind::Unsupported,
        ));
    }
    let referenced = u32::try_from(manifest.layers.len())
        .ok()
        .and_then(|layers| layers.checked_add(1))
        .ok_or_else(descriptor_limit)?;
    *descriptor_count = descriptor_count
        .checked_add(referenced)
        .ok_or_else(descriptor_limit)?;
    if *descriptor_count > request.limits.max_descriptors {
        return Err(descriptor_limit());
    }
    let config = manifest.config.validate(ImportPhase::VerifyConfig)?;
    require_media(&config, CONFIG_MEDIA_TYPE, ImportPhase::VerifyConfig)?;
    let config_bytes = layout.read_blob(&config, document_limit, ImportPhase::VerifyConfig)?;
    let config_wire: ConfigWire = parse_json(&config_bytes, ImportPhase::VerifyConfig)?;
    config_wire.require_supported_platform()?;
    let config_platform = OciPlatform::new(
        config_wire.os,
        config_wire.architecture,
        config_wire.variant,
    )
    .map_err(|_| ImportError::new(ImportPhase::VerifyConfig, ImportErrorKind::InvalidInput))?;
    if config_wire.rootfs.kind != "layers" {
        return Err(ImportError::new(
            ImportPhase::VerifyConfig,
            ImportErrorKind::Unsupported,
        ));
    }
    let platform = effective_platform(&candidate, &config_platform)?;
    let selection_matches = match request.selection {
        OciSelection::Platform(requested) => platform_matches(requested, &platform),
        OciSelection::Exact(identity) => platform_matches(identity.platform(), &platform),
    };
    if !selection_matches {
        return match request.selection {
            OciSelection::Platform(_) => Ok(None),
            OciSelection::Exact(_) => Err(ImportError::new(
                ImportPhase::VerifyConfig,
                ImportErrorKind::Integrity,
            )),
        };
    }
    if manifest.layers.len() != config_wire.rootfs.diff_ids.len() {
        return Err(ImportError::new(
            ImportPhase::VerifyConfig,
            ImportErrorKind::Integrity,
        ));
    }
    let mut layers = Vec::with_capacity(manifest.layers.len());
    for (wire, diff_id) in manifest.layers.into_iter().zip(config_wire.rootfs.diff_ids) {
        let descriptor = wire.validate(ImportPhase::VerifyLayer)?;
        if !matches!(descriptor.media_type.as_str(), PLAIN_LAYER | GZIP_LAYER) {
            return Err(ImportError::new(
                ImportPhase::VerifyLayer,
                ImportErrorKind::Unsupported,
            ));
        }
        if descriptor.size > request.limits.max_blob_bytes {
            return Err(ImportError::new(
                ImportPhase::VerifyLayer,
                ImportErrorKind::LimitExceeded,
            ));
        }
        layers.push((
            descriptor,
            digest::parse(diff_id, ImportPhase::VerifyLayer)?,
        ));
    }
    Ok(Some(SelectedImage {
        candidate,
        manifest_bytes,
        config,
        config_bytes,
        platform,
        layers,
    }))
}

fn effective_platform(
    candidate: &Candidate,
    config: &OciPlatform,
) -> Result<OciPlatform, ImportError> {
    match &candidate.declared_platform {
        Some(declared) => merge_platform_claims(config, declared, ImportPhase::VerifyConfig),
        None => Ok(config.clone()),
    }
}

fn require_media(
    descriptor: &Descriptor,
    expected: &str,
    phase: ImportPhase,
) -> Result<(), ImportError> {
    if descriptor.media_type != expected {
        return Err(ImportError::new(phase, ImportErrorKind::Unsupported));
    }
    Ok(())
}

const fn descriptor_limit() -> ImportError {
    ImportError::new(ImportPhase::VerifyManifest, ImportErrorKind::LimitExceeded)
}
