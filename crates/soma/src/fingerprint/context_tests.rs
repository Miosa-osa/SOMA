use crate::{
    CommandContext, DirectCommand, ExecutionLimits, GenerationId, InstanceId, OciDigest,
    OciPlatform, WorkloadIdentity,
};

use super::execute;

fn fingerprint(command: &DirectCommand) -> String {
    let workload = WorkloadIdentity::new(
        OciDigest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
        OciPlatform::linux_amd64(),
        Some(GenerationId::new(format!("sha256:{}", "b".repeat(64))).expect("generation")),
    );
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
    let limits = ExecutionLimits::new(30_000, 1_024).expect("limits");
    format!("{:?}", execute(&workload, &instance, command, &limits))
}

fn base() -> DirectCommand {
    DirectCommand::new("/bin/true", ["-x"]).expect("command")
}

fn context(environment: &[(&str, &str)], directory: Option<&str>, stdin: &[u8]) -> CommandContext {
    CommandContext::new(
        environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        directory.map(str::to_owned),
        None,
        stdin.to_vec(),
    )
    .expect("context")
}

#[test]
fn an_empty_context_keeps_the_fingerprint_taken_before_contexts_existed() {
    let with_empty = base()
        .with_context(CommandContext::default())
        .expect("context");
    assert_eq!(fingerprint(&base()), fingerprint(&with_empty));
    // Frozen from the commit before contexts existed, so a change here is a change to every
    // idempotency key already stored.
    assert_eq!(fingerprint(&base()), FROZEN_EMPTY_CONTEXT);
}

#[test]
fn every_context_dimension_changes_the_fingerprint() {
    let reference = fingerprint(
        &base()
            .with_context(context(&[("A", "1")], Some("/w"), b"in"))
            .expect("c"),
    );
    let variants = [
        context(&[("A", "2")], Some("/w"), b"in"),
        context(&[("B", "1")], Some("/w"), b"in"),
        context(&[("A", "1")], Some("/x"), b"in"),
        context(&[("A", "1")], None, b"in"),
        context(&[("A", "1")], Some("/w"), b"out"),
        context(&[], Some("/w"), b"in"),
    ];
    for variant in variants {
        let changed = fingerprint(&base().with_context(variant).expect("c"));
        assert_ne!(reference, changed);
    }
}

#[test]
fn environment_order_is_part_of_the_request() {
    let ab = base()
        .with_context(context(&[("A", "1"), ("B", "2")], None, b""))
        .expect("c");
    let ba = base()
        .with_context(context(&[("B", "2"), ("A", "1")], None, b""))
        .expect("c");
    assert_ne!(fingerprint(&ab), fingerprint(&ba));
}

#[test]
fn a_context_that_overflows_the_guest_record_is_refused() {
    let big = "x".repeat(4_096);
    let environment: Vec<(String, String)> =
        (0..64).map(|i| (format!("K{i:03}"), big.clone())).collect();
    let oversized =
        CommandContext::new(environment, None, None, vec![0; 32 * 1024]).expect("bounds hold");
    assert!(base().with_context(oversized).is_err());
}

const FROZEN_EMPTY_CONTEXT: &str = "RequestFingerprint(\"sha256:8c5c5bd4e60d75e0c4c257e402228d196e79899983a23ff54e21e51664446ead\")";
