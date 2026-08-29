use crate::{
    Backend, CleanupEvidence, CleanupMethod, CleanupReason, CleanupRequest, InstanceId, Milestone,
    MilestoneKind, OperationId,
};

use super::{
    Engine, FailurePhase, RunFailure, RunFailureKind,
    run_evidence::{
        CleanupResult, FailureContext, LaunchEvidence, append_failure, append_milestone,
        failure_receipt, failure_receipt_after_launch, times_follow,
    },
};

impl<B: Backend, S> Engine<B, S> {
    pub(super) fn failure_without_cleanup(&self, context: FailureContext) -> RunFailure {
        RunFailure {
            kind: context.kind,
            receipt: Box::new(failure_receipt(
                context,
                self.backend.kind(),
                CleanupEvidence::not_owned(),
            )),
            output: None,
        }
    }

    pub(super) fn failure_with_cleanup(
        &mut self,
        mut context: FailureContext,
        reason: CleanupReason,
        launch: Option<LaunchEvidence>,
    ) -> RunFailure {
        let cleanup = self.perform_cleanup(
            &context.operation_id,
            &context.instance_id,
            reason,
            &mut context.milestones,
        );
        if let Some(kind) = cleanup.failure_kind
            && context.kind == RunFailureKind::CleanupIncomplete
        {
            context.kind = kind;
        }
        let failure_kind = context.kind;
        let receipt = match launch {
            Some(launch) => failure_receipt_after_launch(context, launch, cleanup.evidence),
            None => failure_receipt(context, self.backend.kind(), cleanup.evidence),
        };
        RunFailure {
            kind: failure_kind,
            receipt: Box::new(receipt),
            output: None,
        }
    }

    pub(super) fn perform_cleanup(
        &mut self,
        operation_id: &OperationId,
        instance_id: &InstanceId,
        reason: CleanupReason,
        milestones: &mut Vec<Milestone>,
    ) -> CleanupResult {
        match self
            .backend
            .cleanup(CleanupRequest::new(operation_id, instance_id, reason))
        {
            Ok(observation) => {
                let (observed_operation, observed_instance, evidence, times) =
                    observation.into_parts();
                let [started, finished] = times.values();
                if observed_operation != *operation_id
                    || observed_instance != *instance_id
                    || !cleanup_method_matches(reason, &evidence)
                    || !times_follow(milestones, &[started, finished])
                {
                    return CleanupResult::invalid();
                }
                append_milestone(milestones, MilestoneKind::CleanupStarted, started);
                append_milestone(milestones, MilestoneKind::CleanupFinished, finished);
                CleanupResult {
                    complete: evidence.is_complete(),
                    evidence,
                    failure_kind: None,
                }
            }
            Err(failure) => {
                if !append_failure(milestones, failure) {
                    return CleanupResult::invalid();
                }
                CleanupResult {
                    complete: false,
                    evidence: CleanupEvidence::incomplete_owned_machine(),
                    failure_kind: Some(RunFailureKind::Backend {
                        phase: FailurePhase::Cleanup,
                        kind: failure.kind(),
                    }),
                }
            }
        }
    }
}

fn cleanup_method_matches(reason: CleanupReason, evidence: &CleanupEvidence) -> bool {
    if evidence.all_not_owned() {
        return true;
    }
    matches!(
        (reason, evidence.method()),
        (
            CleanupReason::RunCompleted,
            CleanupMethod::Graceful | CleanupMethod::Forced | CleanupMethod::GracefulThenForced
        ) | (
            CleanupReason::Rollback
                | CleanupReason::ForcedDestroy
                | CleanupReason::UncertainCommandTermination,
            CleanupMethod::Forced | CleanupMethod::GracefulThenForced
        ) | (
            CleanupReason::GracefulStop,
            CleanupMethod::Graceful | CleanupMethod::GracefulThenForced
        )
    )
}
