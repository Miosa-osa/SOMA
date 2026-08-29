use std::fmt::Write as _;

use soma::{OciDigest, OciPlatform, WorkloadIdentity};

use crate::oci::Descriptor;

pub(crate) const IMPORT_MEDIA_TYPE: &str = "application/vnd.soma.generation-input.v1+json";

pub(crate) struct LayerRecord {
    pub(crate) descriptor: Descriptor,
    pub(crate) diff_id: OciDigest,
    pub(crate) expanded_size: u64,
    pub(crate) entry_count: u32,
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
