use super::decode;
use crate::NormalizeErrorKind;

#[test]
fn strict_decoder_accepts_only_the_schema_owned_field_set() {
    assert!(decode(valid().as_bytes()).is_ok());
    for malformed in [
        valid().replace("\"version\":1", "\"version\":1,\"unknown\":0"),
        valid().replace("\"version\":1", "\"version\":1,\"version\":1"),
        valid().replace(
            "\"size\":2},\"config\"",
            "\"size\":2,\"unknown\":0},\"config\"",
        ),
    ] {
        assert_eq!(
            decode(malformed.as_bytes()).err().unwrap().kind(),
            NormalizeErrorKind::Integrity
        );
    }
}

fn valid() -> String {
    let manifest = format!("sha256:{}", "a".repeat(64));
    let config = format!("sha256:{}", "b".repeat(64));
    format!(
        concat!(
            "{{\"format\":\"soma.oci-import\",\"version\":1,",
            "\"workload\":{{\"index_digest\":null,\"manifest_digest\":\"{manifest}\",",
            "\"platform\":{{\"os\":\"linux\",\"architecture\":\"arm64\",\"variant\":null}}}},",
            "\"manifest\":{{\"media_type\":\"application/vnd.oci.image.manifest.v1+json\",",
            "\"digest\":\"{manifest}\",\"size\":2}},",
            "\"config\":{{\"media_type\":\"application/vnd.oci.image.config.v1+json\",",
            "\"digest\":\"{config}\",\"size\":2}},\"layers\":[]}}"
        ),
        manifest = manifest,
        config = config,
    )
}
