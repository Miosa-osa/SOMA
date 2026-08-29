use std::fmt::Write as _;

use serde::Deserialize;
use soma::{OciDigest, OciPlatform, WorkloadIdentity};

use crate::{
    NormalizeError, NormalizeErrorKind, NormalizePhase,
    oci::{CONFIG_MEDIA_TYPE, Descriptor, GZIP_LAYER, MANIFEST_MEDIA_TYPE, PLAIN_LAYER},
};

pub(crate) const IMPORT_MEDIA_TYPE: &str = "application/vnd.soma.generation-input.v1+json";

pub(crate) struct LayerRecord {
    pub(crate) descriptor: Descriptor,
    pub(crate) diff_id: OciDigest,
    pub(crate) expanded_size: u64,
    pub(crate) entry_count: u32,
}

pub(crate) struct DecodedImport {
    pub(crate) workload: WorkloadIdentity,
    pub(crate) layers: Vec<LayerRecord>,
}

pub(crate) fn encode(
    workload: &WorkloadIdentity,
    manifest: &Descriptor,
    config: &Descriptor,
    layers: &[LayerRecord],
) -> Vec<u8> {
    let mut output = String::new();
    output.push_str("{\"format\":\"soma.oci-import\",\"version\":1,\"workload\":{");
    output.push_str("\"index_digest\":");
    match workload.index_digest() {
        Some(value) => quoted(&mut output, value.as_str()),
        None => output.push_str("null"),
    }
    output.push_str(",\"manifest_digest\":");
    quoted(&mut output, workload.manifest_digest().as_str());
    output.push_str(",\"platform\":");
    platform(&mut output, workload.platform());
    output.push_str("},\"manifest\":");
    descriptor(&mut output, manifest);
    output.push_str(",\"config\":");
    descriptor(&mut output, config);
    output.push_str(",\"layers\":[");
    for (ordinal, layer) in layers.iter().enumerate() {
        if ordinal != 0 {
            output.push(',');
        }
        output.push_str("{\"ordinal\":");
        write!(output, "{ordinal}").expect("writing to String cannot fail");
        output.push_str(",\"blob\":");
        descriptor(&mut output, &layer.descriptor);
        output.push_str(",\"diff_id\":");
        quoted(&mut output, layer.diff_id.as_str());
        output.push_str(",\"expanded_size\":");
        write!(output, "{}", layer.expanded_size).expect("writing to String cannot fail");
        output.push_str(",\"entry_count\":");
        write!(output, "{}", layer.entry_count).expect("writing to String cannot fail");
        output.push('}');
    }
    output.push_str("]}");
    output.into_bytes()
}

fn descriptor(output: &mut String, value: &Descriptor) {
    output.push_str("{\"media_type\":");
    quoted(output, &value.media_type);
    output.push_str(",\"digest\":");
    quoted(output, value.digest.as_str());
    output.push_str(",\"size\":");
    write!(output, "{}", value.size).expect("writing to String cannot fail");
    output.push('}');
}

fn platform(output: &mut String, value: &OciPlatform) {
    output.push_str("{\"os\":");
    quoted(output, value.operating_system());
    output.push_str(",\"architecture\":");
    quoted(output, value.architecture());
    output.push_str(",\"variant\":");
    match value.variant() {
        Some(variant) => quoted(output, variant),
        None => output.push_str("null"),
    }
    output.push('}');
}

fn quoted(output: &mut String, value: &str) {
    output.push('"');
    output.push_str(value);
    output.push('"');
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImport, NormalizeError> {
    let wire: ImportWire = serde_json::from_slice(bytes).map_err(|_| manifest_error())?;
    if wire.format != "soma.oci-import" || wire.version != 1 {
        return Err(manifest_error());
    }
    let platform = OciPlatform::new(
        wire.workload.platform.os,
        wire.workload.platform.architecture,
        wire.workload.platform.variant,
    )
    .map_err(|_| manifest_error())?;
    let mut workload = WorkloadIdentity::new(wire.workload.manifest_digest, platform, None);
    if let Some(index) = wire.workload.index_digest {
        workload = workload.with_index_digest(index);
    }
    let selected_manifest = wire.manifest.into_descriptor();
    let config = wire.config.into_descriptor();
    if selected_manifest.media_type != MANIFEST_MEDIA_TYPE
        || selected_manifest.digest != *workload.manifest_digest()
        || config.media_type != CONFIG_MEDIA_TYPE
    {
        return Err(manifest_error());
    }
    let mut layers = Vec::with_capacity(wire.layers.len());
    for (index, layer) in wire.layers.into_iter().enumerate() {
        let ordinal = u32::try_from(index).map_err(|_| manifest_error())?;
        let descriptor = layer.blob.into_descriptor();
        if layer.ordinal != ordinal
            || !matches!(descriptor.media_type.as_str(), PLAIN_LAYER | GZIP_LAYER)
        {
            return Err(manifest_error());
        }
        layers.push(LayerRecord {
            descriptor,
            diff_id: layer.diff_id,
            expanded_size: layer.expanded_size,
            entry_count: layer.entry_count,
        });
    }
    Ok(DecodedImport { workload, layers })
}

const fn manifest_error() -> NormalizeError {
    NormalizeError::new(NormalizePhase::OpenImport, NormalizeErrorKind::Integrity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportWire {
    format: String,
    version: u32,
    workload: WorkloadWire,
    manifest: DescriptorWire,
    config: DescriptorWire,
    layers: Vec<LayerWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkloadWire {
    index_digest: Option<OciDigest>,
    manifest_digest: OciDigest,
    platform: PlatformWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformWire {
    os: String,
    architecture: String,
    variant: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorWire {
    media_type: String,
    digest: OciDigest,
    size: u64,
}

impl DescriptorWire {
    fn into_descriptor(self) -> Descriptor {
        Descriptor {
            media_type: self.media_type,
            digest: self.digest,
            size: self.size,
            platform: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerWire {
    ordinal: u32,
    blob: DescriptorWire,
    diff_id: OciDigest,
    expanded_size: u64,
    entry_count: u32,
}

#[cfg(test)]
mod tests;
