use crate::{RuntimeFailure, RuntimeFailureKind};
use rmcp::{ErrorData, model::CallToolResult};

pub(super) fn runtime_failure_result(
    operation: &'static str,
    operation_id: Option<&crate::OperationId>,
    instance_id: Option<&crate::InstanceId>,
    failure: &RuntimeFailure,
    max_output_bytes: Option<u64>,
) -> CallToolResult {
    crate::result::failure_result(
        operation,
        operation_id,
        instance_id,
        crate::result::FailureDescriptor::new(
            runtime_failure_code(failure),
            runtime_failure_message(failure),
            runtime_failure_retryable(failure),
        ),
        failure,
        max_output_bytes,
    )
    .unwrap_or_else(|_| {
        CallToolResult::error(vec![rmcp::model::ContentBlock::text(
            "SOMA could not encode a bounded failure response",
        )])
    })
}

pub(super) fn wrong_result_type() -> ErrorData {
    ErrorData::internal_error("SOMA runtime returned the wrong result type", None)
}

const fn runtime_failure_code(failure: &RuntimeFailure) -> &'static str {
    match failure.kind() {
        RuntimeFailureKind::Unsupported => "unsupported",
        RuntimeFailureKind::Unavailable => "unavailable",
        RuntimeFailureKind::Rejected => "rejected",
        RuntimeFailureKind::InvalidState => "invalid_state",
        RuntimeFailureKind::NotFound => "not_found",
        RuntimeFailureKind::Conflict => "conflict",
        RuntimeFailureKind::Timeout => "timeout",
        RuntimeFailureKind::OutputLimit => "output_limit",
        RuntimeFailureKind::CleanupIncomplete => "cleanup_incomplete",
        RuntimeFailureKind::Internal => "internal",
    }
}

const fn runtime_failure_message(failure: &RuntimeFailure) -> &'static str {
    match failure.kind() {
        RuntimeFailureKind::Unsupported => "the requested backend is unsupported",
        RuntimeFailureKind::Unavailable => "the requested backend is unavailable",
        RuntimeFailureKind::Rejected => "the workload was rejected by the isolation backend",
        RuntimeFailureKind::InvalidState => "the sandbox is not in the required state",
        RuntimeFailureKind::NotFound => "the sandbox was not found",
        RuntimeFailureKind::Conflict => "the operation conflicts with existing state",
        RuntimeFailureKind::Timeout => "the operation exceeded its deadline",
        RuntimeFailureKind::OutputLimit => "guest output exceeded its allowance",
        RuntimeFailureKind::CleanupIncomplete => "sandbox cleanup could not be proven",
        RuntimeFailureKind::Internal => "the runtime could not complete the operation",
    }
}

/// Whether resubmitting the identical call, with no operator action in between, could succeed.
///
/// `Unavailable` is not one of them. A backend capability this host does not have does not
/// appear because the caller asked a second time, and a client that reads `retryable` as
/// permission to keep asking would retry a permanently fatal condition forever.
const fn runtime_failure_retryable(failure: &RuntimeFailure) -> bool {
    matches!(
        failure.kind(),
        RuntimeFailureKind::Timeout | RuntimeFailureKind::Internal
    )
}
