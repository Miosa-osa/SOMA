//! The decoder applies the validator's shape rules, so a well-formed but invalid lock never
//! reaches the revision view.

mod support;

use soma_template::{LockError, TemplateLock, TemplateRevision};
use support::{EXAMPLE, edit, example, lock, replace_bytes, replace_nth};

fn rejected(bytes: &[u8], field: &'static str) {
    assert_eq!(
        TemplateLock::decode(bytes),
        Err(LockError::InvalidField { field })
    );
}

fn resources_offset(bytes: &[u8]) -> usize {
    let marker: Vec<u8> = [
        2_u32.to_be_bytes().as_slice(),
        &2048_u64.to_be_bytes(),
        &10_240_u64.to_be_bytes(),
    ]
    .concat();
    bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("resources are encoded in order")
}

fn lifecycle_offset(bytes: &[u8]) -> usize {
    let marker: Vec<u8> = [
        900_u64.to_be_bytes().as_slice(),
        &14_400_u64.to_be_bytes(),
        &[0],
    ]
    .concat();
    bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("lifecycle is encoded in order")
}

#[test]
fn command_environment_and_secret_text_must_keep_their_shape() {
    let bytes = example().encode();
    assert!(TemplateLock::decode(&bytes).is_ok());
    let cases: [(&str, &str, bool, &'static str); 9] = [
        (
            "\x00\x00\x00\x06claude",
            "\x00\x00\x00\x06cl\0ude",
            false,
            "command",
        ),
        (
            "\x00\x00\x00\x04root",
            "\x00\x00\x00\x04r\not",
            false,
            "command",
        ),
        (
            "\x00\x00\x00\x0a/workspace",
            "\x00\x00\x00\x0aworkspace/",
            false,
            "command",
        ),
        (
            "\x00\x00\x00\x02CI",
            "\x00\x00\x00\x02C=",
            false,
            "environment",
        ),
        (
            "\x00\x00\x00\x04true",
            "\x00\x00\x00\x04tr\0e",
            false,
            "environment",
        ),
        ("ANTHROPIC_API_KEY", "=NTHROPIC_API_KEY", true, "secrets"),
        ("ANTHROPIC_API_KEY", "=NTHROPIC_API_KEY", false, "secrets"),
        ("secret://", "secret:/x", false, "secrets"),
        (
            "\x00\x00\x00\x05amd64",
            "\x00\x00\x00\x05s390x",
            false,
            "workload.platform",
        ),
    ];
    for (from, to, last, field) in cases {
        rejected(&replace_bytes(&bytes, from, to, last), field);
    }
}

#[test]
fn numeric_fields_must_stay_within_the_validator_bounds() {
    let bytes = example().encode();
    let resources = resources_offset(&bytes);
    let lifecycle = lifecycle_offset(&bytes);
    let mut zero_vcpus = bytes.clone();
    zero_vcpus[resources..resources + 4].copy_from_slice(&0_u32.to_be_bytes());
    rejected(&zero_vcpus, "resources");
    let mut wide = bytes.clone();
    wide[resources..resources + 4].copy_from_slice(&70_000_u32.to_be_bytes());
    rejected(&wide, "resources");
    let mut memory = bytes.clone();
    memory[resources + 4..resources + 12].copy_from_slice(&16_385_u64.to_be_bytes());
    rejected(&memory, "resources");
    let mut idle = bytes.clone();
    idle[lifecycle..lifecycle + 8].copy_from_slice(&20_000_u64.to_be_bytes());
    rejected(&idle, "lifecycle");
    let mut maximum = bytes.clone();
    maximum[lifecycle + 8..lifecycle + 16].copy_from_slice(&0_u64.to_be_bytes());
    rejected(&maximum, "lifecycle");
    let mut unsupported = bytes.clone();
    unsupported[lifecycle + 16] = 2;
    rejected(&unsupported, "lifecycle");
    let mut mode = lock(&edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"file\"\nscope = \"/run/key\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    ))
    .encode();
    let position = mode
        .windows(5)
        .position(|window| window == [1, 0, 0, 1, 0])
        .expect("presence byte then mode 0o400");
    mode[position + 1..position + 5].copy_from_slice(&0o644_u32.to_be_bytes());
    rejected(&mode, "secrets");
}

#[test]
fn a_proxy_scope_outside_the_envelope_is_rejected() {
    let bytes = lock(&edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"egress-proxy\"\nscope = \"api.anthropic.com\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    ))
    .encode();
    assert!(TemplateLock::decode(&bytes).is_ok());
    let outside = replace_nth(&bytes, b"api.anthropic.com", b"api.anthropic.org", 1);
    rejected(&outside, "secrets");
}

#[test]
fn every_decoded_lock_projects_onto_the_view() {
    let bytes = example().encode();
    let decoded = TemplateLock::decode(&bytes).expect("canonical");
    let view = TemplateRevision::from_lock(&decoded);
    assert_eq!(view.lock_id(), decoded.id());
    assert!(!view.default_command().program().contains('\0'));
    assert_ne!(view.ttl_seconds(), 0);
}
