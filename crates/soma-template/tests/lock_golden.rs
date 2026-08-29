//! Golden bytes and identity of the specification example lock.

mod support;

use soma_template::{
    LOCK_MAGIC, LOCK_SCHEMA_VERSION, LockId, LockedEnvironment, POLICY_VERSION, TemplateLock,
};
use support::{EXAMPLE, PYTHON_DIGEST, example, hex, lock, unhex};

const GOLDEN_HEX: &str = include_str!("fixtures/example-lock.hex");
const GOLDEN_ID: &str = include_str!("fixtures/example-lock.id");

#[test]
fn example_lock_bytes_match_golden() {
    let bytes = example().encode();
    assert_eq!(hex(&bytes), hex(&unhex(GOLDEN_HEX)));
    assert_eq!(LockId::of(&bytes).to_string(), GOLDEN_ID.trim());
}

#[test]
fn repeated_resolution_yields_identical_bytes_and_identity() {
    let first = example();
    let second = lock(EXAMPLE);
    assert_eq!(first, second);
    assert_eq!(first.encode(), second.encode());
    assert_eq!(first.id(), second.id());
}

#[test]
fn lock_starts_with_magic_schema_and_policy_version() {
    let bytes = example().encode();
    assert_eq!(&bytes[..8], LOCK_MAGIC);
    assert_eq!(&bytes[8..10], LOCK_SCHEMA_VERSION.to_be_bytes());
    let schema = "soma.template/v1alpha1";
    let length = u32::try_from(schema.len()).expect("short");
    assert_eq!(&bytes[10..14], length.to_be_bytes());
    assert_eq!(&bytes[14..14 + schema.len()], schema.as_bytes());
    let policy = 14 + schema.len();
    assert_eq!(&bytes[policy..policy + 2], POLICY_VERSION.to_be_bytes());
}

#[test]
fn lock_round_trips_through_the_hostile_decoder() {
    let lock = example();
    let bytes = lock.encode();
    let decoded = TemplateLock::decode(&bytes).expect("canonical bytes decode");
    assert_eq!(decoded, lock);
    assert_eq!(decoded.encode(), bytes);
    assert_eq!(decoded.id(), lock.id());
}

#[test]
fn lock_binds_exact_digest_and_excludes_non_content_fields() {
    let lock = example();
    assert_eq!(lock.image().digest().as_str(), PYTHON_DIGEST);
    let bytes = lock.encode();
    let contains = |needle: &str| {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    assert!(contains("secret://anthropic/default"));
    assert!(contains("api.anthropic.com"));
    assert!(!contains("claude-code-python"), "name is not content");
    assert!(
        !contains("python:3.12-slim"),
        "mutable reference is not content"
    );
}

#[test]
fn lock_records_the_resolved_composition() {
    let lock = example();
    let modules: Vec<String> = lock
        .modules()
        .iter()
        .map(|module| module.identity().to_string())
        .collect();
    assert_eq!(
        modules,
        ["soma://agent/claude-code@1", "soma://tools/git@1"]
    );
    assert_eq!(lock.command().program(), "claude");
    assert_eq!(lock.command().working_directory(), "/workspace");
    assert_eq!(lock.command().user(), "root");
    assert_eq!(lock.resources().vcpus, 2);
    assert_eq!(lock.lifecycle().maximum_lifetime_seconds, 14_400);
    let names: Vec<&str> = lock
        .environment()
        .iter()
        .map(LockedEnvironment::name)
        .collect();
    assert_eq!(names, ["CI", "GIT_TERMINAL_PROMPT"]);
    let sealed = &lock.environment()[1];
    assert_eq!(sealed.value(), Some("0"));
    assert_eq!(
        sealed.sealed_by().map(ToString::to_string).as_deref(),
        Some("soma://tools/git@1")
    );
    assert_eq!(lock.secrets().len(), 1);
    assert_eq!(lock.secrets()[0].scope(), "ANTHROPIC_API_KEY");
    assert_eq!(lock.secrets()[0].mode(), None);
}

#[test]
fn lock_identity_parses_and_displays() {
    let id = example().id();
    let text = id.to_string();
    assert!(text.starts_with("sha256:"));
    assert_eq!(text.len(), 7 + 64);
    assert_eq!(LockId::parse(&text).expect("canonical"), id);
    assert!(LockId::parse("sha256:abc").is_err());
    assert!(LockId::parse(&text.to_uppercase()).is_err());
    assert!(LockId::parse(&text[7..]).is_err());
}
