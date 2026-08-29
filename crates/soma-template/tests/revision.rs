//! The `TemplateRevision` view maps onto the Generation compiler's input contract.

mod support;

use soma::{EgressPolicy, NetworkPolicy, OciImage};
use soma_template::{
    EgressIntent, IngressIntent, InvalidReason, ModuleRegistry, PolicyCeiling, Rejection,
    RejectionClass, RevisionError, TemplateRevision, resolve,
};
use support::{
    EXAMPLE, PYTHON_DIGEST, PYTHON_SIZE, assert_names, backend, edit, example, lock, oracle, parse,
    resolver,
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
fn provenance_attaches_only_a_document_that_composes_to_the_locked_selection() {
    let lock = example();
    let registry = ModuleRegistry::builtin();
    let revision = TemplateRevision::from_lock(&lock)
        .with_provenance(&parse(EXAMPLE), &registry)
        .expect("same document");
    assert_eq!(
        revision.image().reference().map(OciImage::as_str),
        Some("python:3.12-slim")
    );
    let renamed = parse(&edit(EXAMPLE, "claude-code-python", "renamed"));
    assert!(
        TemplateRevision::from_lock(&lock)
            .with_provenance(&renamed, &registry)
            .is_ok()
    );
    let retagged = parse(&edit(EXAMPLE, "python:3.12-slim", "python:3.12"));
    let attached = TemplateRevision::from_lock(&lock)
        .with_provenance(&retagged, &registry)
        .expect("the reference text is provenance the check cannot see");
    assert_eq!(
        attached.image().reference().map(OciImage::as_str),
        Some("python:3.12")
    );
    let mismatch = |text: &str| {
        TemplateRevision::from_lock(&lock)
            .with_provenance(&parse(text), &registry)
            .err()
    };
    assert_eq!(
        mismatch(&edit(EXAMPLE, "memory_mib = 2048", "memory_mib = 1024")),
        Some(RevisionError::ProvenanceMismatch)
    );
    assert_eq!(
        mismatch(&edit(EXAMPLE, "\"soma://tools/git@1\",", "")),
        Some(RevisionError::ProvenanceMismatch)
    );
    assert_eq!(
        mismatch(&edit(EXAMPLE, "value = \"true\"", "value = \"false\"")),
        Some(RevisionError::ProvenanceMismatch)
    );
    let uncomposable = edit(
        EXAMPLE,
        "\"soma://tools/git@1\",",
        "\"soma://tools/git@1\",\n  \"soma://tools/nope@1\",",
    );
    assert_eq!(
        mismatch(&uncomposable),
        Some(RevisionError::ProvenanceMismatch)
    );
    assert!(
        TemplateRevision::from_lock(&lock)
            .with_provenance(&parse(EXAMPLE), &ModuleRegistry::empty())
            .is_err(),
        "a registry that cannot compose the document proves nothing"
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
fn the_lifetime_bound_matches_the_compiler_contract() {
    let thirty_days = 30 * 24 * 60 * 60;
    let accepted = edit(
        EXAMPLE,
        "maximum_lifetime_seconds = 14400",
        &format!("maximum_lifetime_seconds = {thirty_days}"),
    );
    assert_eq!(
        TemplateRevision::from_lock(&lock(&accepted)).ttl_seconds(),
        thirty_days
    );
    let rejected = edit(
        EXAMPLE,
        "maximum_lifetime_seconds = 14400",
        &format!("maximum_lifetime_seconds = {}", thirty_days + 1),
    );
    let rejection = support::reject(&rejected);
    assert_names(
        &rejection,
        RejectionClass::InvalidValue,
        "lifecycle.maximum_lifetime_seconds",
        None,
    );
    assert!(matches!(
        rejection,
        Rejection::InvalidValue {
            reason: InvalidReason::InvalidTimeout,
            ..
        }
    ));
}
