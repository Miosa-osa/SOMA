//! Which changes move the lock identity and which do not.

mod support;

use soma::OciPlatform;
use soma_template::{
    BackendCapabilities, EgressIntent, IdleAction, IngressIntent, PolicyCeiling, ResourceLimits,
    TestResolver, parse_template, resolve,
};
use support::{
    EXAMPLE, OTHER_DIGEST, PYTHON_SIZE, backend, ceiling, digest, edit, example, lock, oracle,
    resolver,
};

#[test]
fn reordering_modules_changes_identity() {
    let reordered = edit(
        EXAMPLE,
        "\"soma://agent/claude-code@1\",\n  \"soma://tools/git@1\",",
        "\"soma://tools/git@1\",\n  \"soma://agent/claude-code@1\",",
    );
    let baseline = example();
    let swapped = lock(&reordered);
    assert_ne!(baseline.id(), swapped.id());
    assert_ne!(baseline.encode(), swapped.encode());
    let order: Vec<String> = swapped
        .modules()
        .iter()
        .map(|module| module.identity().to_string())
        .collect();
    assert_eq!(order, ["soma://tools/git@1", "soma://agent/claude-code@1"]);
}

#[test]
fn renaming_does_not_change_identity() {
    let renamed = edit(
        EXAMPLE,
        "name = \"claude-code-python\"",
        "name = \"another-name\"\ndescription = \"free text provenance\"",
    );
    assert_eq!(example().id(), lock(&renamed).id());
}

#[test]
fn formatting_and_key_order_do_not_change_identity() {
    let restyled = EXAMPLE
        .replace(
            "[workload]\nimage = \"python:3.12-slim\"\nplatform = \"linux/amd64\"",
            "",
        )
        .replace(
            "[command]",
            "# comment\nworkload = { platform = \"linux/amd64\", image = \"python:3.12-slim\" }\n\n[command]",
        )
        .replace("args = []\n", "")
        .replace(
            "working_directory = \"/workspace\"",
            "working_directory = '/workspace'\nargs = []",
        );
    assert_eq!(example().id(), lock(&restyled).id());
}

#[test]
fn equivalent_network_intent_normalizes_to_one_identity() {
    let allowlist = edit(EXAMPLE, "egress = \"deny\"", "egress = \"allowlist\"");
    assert_eq!(example().id(), lock(&allowlist).id());
    let two = edit(
        EXAMPLE,
        "allow_domains = [\"api.anthropic.com\"]",
        "allow_domains = [\"github.com\", \"api.anthropic.com\", \"github.com\"]",
    );
    let swapped = edit(
        EXAMPLE,
        "allow_domains = [\"api.anthropic.com\"]",
        "allow_domains = [\"api.anthropic.com\", \"github.com\"]",
    );
    assert_eq!(lock(&two).id(), lock(&swapped).id());
    assert_ne!(lock(&two).id(), example().id());
}

#[test]
fn every_content_field_moves_identity() {
    let baseline = example().id();
    let edits = [
        ("memory_mib = 2048", "memory_mib = 4096"),
        ("vcpus = 2", "vcpus = 1"),
        (
            "writable_storage_mib = 10240",
            "writable_storage_mib = 8192",
        ),
        ("program = \"claude\"", "program = \"python3\""),
        ("args = []", "args = [\"--verbose\"]"),
        (
            "working_directory = \"/workspace\"",
            "working_directory = \"/\"",
        ),
        ("idle_timeout_seconds = 900", "idle_timeout_seconds = 600"),
        (
            "maximum_lifetime_seconds = 14400",
            "maximum_lifetime_seconds = 3600",
        ),
        ("on_idle = \"destroy\"", "on_idle = \"stop\""),
        ("value = \"true\"", "value = \"false\""),
        ("name = \"CI\"", "name = \"CI_MODE\""),
        (
            "source = \"secret://anthropic/default\"",
            "source = \"secret://anthropic/other\"",
        ),
        (
            "delivery = \"environment\"",
            "delivery = \"file\"\nscope = \"/run/secrets/key\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
        ),
        ("\"soma://tools/git@1\",", ""),
        ("platform = \"linux/amd64\"", "platform = \"linux/arm64\""),
    ];
    for (from, to) in edits {
        let changed = lock(&edit(EXAMPLE, from, to)).id();
        assert_ne!(baseline, changed, "`{to}` must change the identity");
    }
}

#[test]
fn policy_ceiling_and_backend_are_lock_inputs() {
    let template = parse_template(EXAMPLE.as_bytes()).expect("parses");
    let baseline = example().id();
    let wider = PolicyCeiling::new(EgressIntent::Unrestricted, IngressIntent::Unrestricted);
    let with_ceiling =
        resolve(&template, &resolver(), &wider, &backend(), &oracle()).expect("resolves");
    assert_ne!(baseline, with_ceiling.id());
    let smaller = BackendCapabilities::new(
        &[OciPlatform::linux_amd64()],
        &[IdleAction::Destroy],
        ResourceLimits {
            max_vcpus: 4,
            max_memory_mib: 8_192,
            max_writable_storage_mib: 32_768,
        },
    )
    .expect("bounded");
    let with_backend =
        resolve(&template, &resolver(), &ceiling(), &smaller, &oracle()).expect("resolves");
    assert_ne!(baseline, with_backend.id());
}

#[test]
fn resolved_digest_is_bound() {
    let template = parse_template(EXAMPLE.as_bytes()).expect("parses");
    let moved = TestResolver::new().with_image(
        "python:3.12-slim",
        &OciPlatform::linux_amd64(),
        digest(OTHER_DIGEST),
        PYTHON_SIZE,
    );
    let relocked = resolve(&template, &moved, &ceiling(), &backend(), &oracle()).expect("resolves");
    assert_ne!(example().id(), relocked.id());
    assert_eq!(relocked.image().digest().as_str(), OTHER_DIGEST);
}

#[test]
fn explicit_defaults_do_not_change_identity() {
    let baseline = example().id();
    let explicit = edit(
        &edit(
            EXAMPLE,
            "working_directory = \"/workspace\"",
            "working_directory = \"/workspace\"\nuser = \"root\"",
        ),
        "delivery = \"environment\"",
        "delivery = \"environment\"\nscope = \"ANTHROPIC_API_KEY\"",
    );
    assert_eq!(baseline, lock(&explicit).id());
    let file = edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"file\"\nscope = \"/run/key\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    );
    let file_mode = edit(
        &file,
        "scope = \"/run/key\"",
        "scope = \"/run/key\"\nmode = 256",
    );
    assert_eq!(lock(&file).id(), lock(&file_mode).id());
    let root_dir = edit(
        EXAMPLE,
        "working_directory = \"/workspace\"",
        "working_directory = \"/\"",
    );
    let no_dir = edit(EXAMPLE, "working_directory = \"/workspace\"\n", "");
    assert_eq!(lock(&root_dir).id(), lock(&no_dir).id());
}
