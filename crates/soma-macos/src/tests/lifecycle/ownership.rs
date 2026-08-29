use crate::{BackendError, CommandFailureReason, OwnershipFailure};

use super::super::fixtures::{INSTANCE, backend, control_limits, instance, success};

#[test]
fn existing_instance_operations_fail_closed_before_mutation_without_exact_ownership() {
    let cases = [
        (b"not-json".to_vec(), OwnershipFailure::InvalidJson),
        (b"[]".to_vec(), OwnershipFailure::MissingRecord),
        (
            format!(
                r#"[{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{"io.miosa.soma.instance":"{INSTANCE}"}}}},"id":"soma-{INSTANCE}"}},{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{"io.miosa.soma.instance":"{INSTANCE}"}}}},"id":"soma-{INSTANCE}"}}]"#
            )
            .into_bytes(),
            OwnershipFailure::MultipleRecords,
        ),
        (
            format!(
                r#"[{{"configuration":{{"id":"soma-wrong","labels":{{"io.miosa.soma.instance":"{INSTANCE}"}}}},"id":"soma-wrong"}}]"#
            )
            .into_bytes(),
            OwnershipFailure::NameMismatch,
        ),
        (
            format!(
                r#"[{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{}}}},"id":"soma-{INSTANCE}"}}]"#
            )
            .into_bytes(),
            OwnershipFailure::MissingLabel,
        ),
        (
            format!(
                r#"[{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{"io.miosa.soma.instance":"wrong"}}}},"id":"soma-{INSTANCE}"}}]"#
            )
            .into_bytes(),
            OwnershipFailure::LabelMismatch,
        ),
    ];

    for (document, expected) in cases {
        let (backend, runner) = backend([Ok(success(document))]);

        let error = backend
            .start(instance(), control_limits())
            .expect_err("ownership must be exact");

        assert!(matches!(
            error,
            BackendError::Command { failure }
                if failure.reason() == CommandFailureReason::Ownership(expected)
        ));
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments[0], "inspect");
    }
}
