use soma::GenerationId;

use super::artifacts::Sha256Digest;

/// Derives the `GenerationId` from exact canonical `SOMAGEN` manifest bytes.
///
/// The identity is `sha256:` plus the SHA-256 digest of the manifest bytes and nothing else.
/// This module performs derivation only; it never inspects, validates, or publishes the manifest.
///
/// # Panics
///
/// Cannot panic: a SHA-256 output is never all zero in practice and always renders as a
/// canonical `sha256:` identity.
#[must_use]
pub fn derive_generation_id(manifest_bytes: &[u8]) -> GenerationId {
    let digest = Sha256Digest::of(manifest_bytes);
    GenerationId::new(digest.to_string())
        .expect("a SHA-256 digest of a non-empty manifest is a canonical nonzero identity")
}

/// Returns the raw digest that a `GenerationId` names.
#[must_use]
pub fn generation_id_digest(id: &GenerationId) -> Sha256Digest {
    digest_of(id.as_str())
}

/// Returns the raw digest a canonical `sha256:` identity names.
///
/// # Panics
///
/// Cannot panic for a `GenerationId` or `CandidateId`, whose constructors already enforce the
/// exact `sha256:` plus 64 lowercase hex form.
pub(super) fn digest_of(identity: &str) -> Sha256Digest {
    let hex = identity
        .strip_prefix("sha256:")
        .expect("a content identity always has a sha256 prefix");
    let mut value = [0_u8; 32];
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        value[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    Sha256Digest::from_bytes(value)
}

const fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}
