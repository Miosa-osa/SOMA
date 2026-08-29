//! The `TemplateRevision` view maps onto the Generation compiler's input contract.

mod support;

use soma::{EgressPolicy, NetworkPolicy, OciImage};
use soma_template::{
    EgressIntent, IngressIntent, PolicyCeiling, RevisionError, TemplateLock, TemplateRevision,
    resolve,
};
use support::{
    EXAMPLE, PYTHON_DIGEST, PYTHON_SIZE, backend, edit, example, lock, oracle, parse, resolver,
};

#[test]
fn view_mirrors_the_lock() {
    let lock = example();
    let revision = TemplateRevision::from_lock(&lock);
    assert_eq!(revision.lock_id(), lock.id());
    assert_eq!(revision.content_digest(), lock.content_digest());
    assert_eq!(revision.policy_version(), lock.policy_version());
    assert_eq!(revision.image().manifest_digest().as_str(), PYTHON_DIGEST);
    assert_eq!(revision.image().platform(), lock.image().platform());
    assert_eq!(revision.image().reference(), None);
    assert_eq!(lock.image().size(), PYTHON_SIZE);
    assert_eq!(revision.vcpus(), 2);
    assert_eq!(revision.memory_mib(), 2048);
    assert_eq!(revision.writable_storage_mib(), 10_240);
    assert_eq!(revision.ttl_seconds(), 14_400);
    assert_eq!(revision.lifecycle(), lock.lifecycle());
    assert_eq!(revision.modules(), lock.modules());
    assert_eq!(revision.default_command(), lock.command());
    assert_eq!(revision.environment(), lock.environment());
    assert_eq!(revision.secrets(), lock.secrets());
    assert_eq!(revision.network(), lock.network());
}

#[test]
fn provenance_attaches_only_the_matching_document() {
    let lock = example();
    let revision = TemplateRevision::from_lock(&lock)
        .with_provenance(&parse(EXAMPLE))
        .expect("same document");
    assert_eq!(
        revision.image().reference().map(OciImage::as_str),
        Some("python:3.12-slim")
    );
    let renamed = parse(&edit(EXAMPLE, "claude-code-python", "renamed"));
    assert!(
        TemplateRevision::from_lock(&lock)
            .with_provenance(&renamed)
            .is_ok()
    );
    let changed = parse(&edit(EXAMPLE, "memory_mib = 2048", "memory_mib = 1024"));
    assert_eq!(
        TemplateRevision::from_lock(&lock)
            .with_provenance(&changed)
            .err(),
        Some(RevisionError::ProvenanceMismatch)
    );
}

#[test]
fn shape_is_exact_for_denied_and_unrestricted_envelopes() {
    let denied = lock(&edit(
        EXAMPLE,
        "allow_domains = [\"api.anthropic.com\"]\n",
        "",
    ));
    let shape = TemplateRevision::from_lock(&denied)
        .shape()
        .expect("isolated shape");
    assert_eq!(shape.vcpu_count(), 2);
    assert_eq!(shape.memory_mib(), 2048);
    assert_eq!(shape.storage_mib(), 10_240);
    assert_eq!(
        *shape.capabilities().network_policy(),
        NetworkPolicy::isolated()
    );
    let text = edit(
        &edit(EXAMPLE, "allow_domains = [\"api.anthropic.com\"]\n", ""),
        "egress = \"deny\"",
        "egress = \"unrestricted\"",
    );
    let open = PolicyCeiling::new(EgressIntent::Unrestricted, IngressIntent::Unrestricted);
    let unrestricted =
        resolve(&parse(&text), &resolver(), &open, &backend(), &oracle()).expect("resolves");
    let shape = TemplateRevision::from_lock(&unrestricted)
        .shape()
        .expect("unrestricted shape");
    assert_eq!(
        shape.capabilities().network_policy().egress(),
        EgressPolicy::Unrestricted
    );
}

#[test]
fn shape_fails_closed_for_envelopes_the_portable_contract_cannot_state() {
    let allowlist = TemplateRevision::from_lock(&example());
    assert_eq!(
        allowlist.shape().err(),
        Some(RevisionError::UnrepresentableNetwork)
    );
    let text = edit(
        &edit(EXAMPLE, "allow_domains = [\"api.anthropic.com\"]\n", ""),
        "ingress = \"deny\"",
        "ingress = \"unrestricted\"",
    );
    let open = PolicyCeiling::new(EgressIntent::Unrestricted, IngressIntent::Unrestricted);
    let ingress =
        resolve(&parse(&text), &resolver(), &open, &backend(), &oracle()).expect("resolves");
    assert_eq!(
        TemplateRevision::from_lock(&ingress).shape().err(),
        Some(RevisionError::UnrepresentableNetwork)
    );
}

#[test]
fn shape_fails_closed_for_a_decoded_lock_with_a_zero_dimension() {
    let mut bytes = example().encode();
    let marker: Vec<u8> = [
        2_u32.to_be_bytes().as_slice(),
        &2048_u64.to_be_bytes(),
        &10_240_u64.to_be_bytes(),
    ]
    .concat();
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("resources are encoded in order");
    bytes[offset..offset + 4].copy_from_slice(&0_u32.to_be_bytes());
    let decoded = TemplateLock::decode(&bytes).expect("structurally valid");
    assert_eq!(decoded.resources().vcpus, 0);
    assert_eq!(
        TemplateRevision::from_lock(&decoded).shape().err(),
        Some(RevisionError::InvalidShape)
    );
    let huge = 70_000_u32;
    bytes[offset..offset + 4].copy_from_slice(&huge.to_be_bytes());
    let decoded = TemplateLock::decode(&bytes).expect("structurally valid");
    assert_eq!(
        TemplateRevision::from_lock(&decoded).shape().err(),
        Some(RevisionError::InvalidShape)
    );
}
