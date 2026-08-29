use crate::{
    BackendFailure, BackendKind, CapturedOutput, CleanupEvidence, CommandObservation,
    CommandStatus, DigestBinding, EffectiveNetwork, EffectiveShape, ExecutionReceipt, InstanceId,
    IsolationClass, LaunchObservation, MeasurementBoundary, MeasurementClass, Milestone,
    MilestoneKind, Observation, ObservationUnavailable, OperationId, OperationKind, OutputMetadata,
    PreparationClass, RequestFingerprint, StreamMetadata, TerminalStatus, WorkloadEvidence,
    WorkloadIdentity, fingerprint,
};

use super::{RunFailureKind, RunFailureKind::ObservationMismatch};

#[derive(Clone)]
pub(super) struct LaunchEvidence {
    pub(super) backend: BackendKind,
    pub(super) isolation: IsolationClass,
    pub(super) preparation: PreparationClass,
    pub(super) digest_binding: DigestBinding,
    pub(super) effective_shape: EffectiveShape,
    pub(super) effective_network: EffectiveNetwork,
    pub(super) times: crate::LaunchTimes,
}

#[allow(
    clippy::too_many_arguments,
    reason = "launch validation compares every requested and observed identity dimension"
)]
pub(super) fn validate_launch(
    observation: LaunchObservation,
    operation_id: &OperationId,
    instance_id: &InstanceId,
    workload: &WorkloadIdentity,
    backend: BackendKind,
    requested_shape: &crate::MachineShape,
    previous_time: u64,
) -> Option<LaunchEvidence> {
    let crate::backend::LaunchObservationParts {
        operation_id: observed_operation,
        instance_id: observed_instance,
        workload: observed_workload,
        backend: observed_backend,
        isolation,
        preparation,
        digest_binding,
        effective_shape,
        effective_network,
        times,
    } = observation.into_parts();
    let values = times.values();
    (observed_operation == *operation_id
        && observed_instance == *instance_id
        && observed_workload == *workload
        && observed_backend == backend
        && effective_shape.matches_request(requested_shape)
        && effective_network.matches_request(requested_shape.capabilities().network_policy())
        && values[0] >= previous_time
        && values.windows(2).all(|pair| pair[0] <= pair[1]))
    .then_some(LaunchEvidence {
        backend,
        isolation,
        preparation,
        digest_binding,
        effective_shape,
        effective_network,
        times,
    })
}

pub(super) struct ValidatedCommand {
    pub(super) status: CommandStatus,
    pub(super) output: CapturedOutput,
    pub(super) metadata: OutputMetadata,
    pub(super) times: crate::CommandTimes,
}

pub(super) fn validate_command(
    observation: CommandObservation,
    operation_id: &OperationId,
    instance_id: &InstanceId,
    limits: &crate::ExecutionLimits,
    previous_time: u64,
) -> Option<ValidatedCommand> {
    let (observed_operation, observed_instance, status, output, times) = observation.into_parts();
    let [started, finished] = times.values();
    if observed_operation != *operation_id
        || observed_instance != *instance_id
        || started < previous_time
        || finished < started
    {
        return None;
    }
    let (stdout, stdout_observed, stderr, stderr_observed) = output.into_parts();
    let stdout_captured = u64::try_from(stdout.len()).ok()?;
    let stderr_captured = u64::try_from(stderr.len()).ok()?;
    let captured = stdout_captured.checked_add(stderr_captured)?;
    let observed = stdout_observed.checked_add(stderr_observed)?;
    let within_capture = captured <= limits.max_output_bytes()
        && stdout_observed >= stdout_captured
        && stderr_observed >= stderr_captured;
    let exceeded = observed > limits.max_output_bytes();
    if !within_capture || (exceeded != matches!(status, CommandStatus::OutputLimitExceeded)) {
        return None;
    }
    let metadata = OutputMetadata::new(
        StreamMetadata::new(
            stdout_captured,
            stdout_observed,
            stdout_observed > stdout_captured,
            fingerprint::bytes(&stdout),
        ),
        StreamMetadata::new(
            stderr_captured,
            stderr_observed,
            stderr_observed > stderr_captured,
            fingerprint::bytes(&stderr),
        ),
    );
    Some(ValidatedCommand {
        status,
        output: CapturedOutput::new(stdout, stderr),
        metadata,
        times,
    })
}

pub(super) fn append_launch(milestones: &mut Vec<Milestone>, times: crate::LaunchTimes) {
    let [admitted, launched, ready] = times.values();
    append_milestone(milestones, MilestoneKind::Admitted, admitted);
    append_milestone(milestones, MilestoneKind::MachineLaunched, launched);
    append_milestone(milestones, MilestoneKind::Ready, ready);
}

pub(super) fn append_command(milestones: &mut Vec<Milestone>, times: crate::CommandTimes) {
    let [started, finished] = times.values();
    append_milestone(milestones, MilestoneKind::CommandStarted, started);
    append_milestone(milestones, MilestoneKind::CommandFinished, finished);
}

pub(super) fn append_failure(milestones: &mut Vec<Milestone>, failure: BackendFailure) -> bool {
    append_milestone(
        milestones,
        MilestoneKind::FailureObserved,
        failure.occurred_at_ns(),
    )
}

pub(super) fn append_milestone(
    milestones: &mut Vec<Milestone>,
    kind: MilestoneKind,
    elapsed_ns: u64,
) -> bool {
    if milestones
        .last()
        .is_some_and(|previous| elapsed_ns < previous.elapsed_ns())
    {
        return false;
    }
    milestones.push(Milestone::new(kind, elapsed_ns));
    true
}

pub(super) fn times_follow(milestones: &[Milestone], values: &[u64]) -> bool {
    let previous = milestones.last().map_or(0, Milestone::elapsed_ns);
    values.first().is_none_or(|first| *first >= previous)
        && values.windows(2).all(|pair| pair[0] <= pair[1])
}

pub(super) fn terminal_status(status: CommandStatus) -> TerminalStatus {
    match status {
        CommandStatus::Exited { code } => TerminalStatus::Exited { code },
        CommandStatus::Signaled { signal } => TerminalStatus::Signaled { signal },
        CommandStatus::TimedOut => TerminalStatus::TimedOut,
        CommandStatus::OutputLimitExceeded => TerminalStatus::OutputLimitExceeded,
    }
}

pub(super) struct FailureContext {
    pub(super) kind: RunFailureKind,
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) machine_name: Option<crate::MachineName>,
    pub(super) fingerprint: RequestFingerprint,
    pub(super) workload: WorkloadEvidence,
    pub(super) requested_shape: crate::MachineShape,
    pub(super) milestones: Vec<Milestone>,
}

pub(super) struct ReceiptContext {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) machine_name: Option<crate::MachineName>,
    pub(super) fingerprint: RequestFingerprint,
    pub(super) workload: WorkloadIdentity,
    pub(super) requested_shape: crate::MachineShape,
    pub(super) launch: LaunchEvidence,
    pub(super) milestones: Vec<Milestone>,
    pub(super) terminal_status: TerminalStatus,
    pub(super) output: OutputMetadata,
    pub(super) cleanup: CleanupEvidence,
}

pub(super) fn successful_receipt(context: ReceiptContext) -> ExecutionReceipt {
    ExecutionReceipt::new(
        OperationKind::Run,
        context.operation_id,
        context.instance_id,
        context.machine_name,
        context.fingerprint,
        WorkloadEvidence::Resolved {
            identity: context.workload,
        },
        context.launch.backend,
        Observation::Observed(context.launch.isolation),
        Observation::Observed(context.launch.preparation),
        Observation::Observed(context.launch.digest_binding),
        context.requested_shape,
        context.launch.effective_shape,
        context.launch.effective_network,
        context.milestones,
        context.terminal_status,
        Observation::Observed(context.output),
        context.cleanup,
        MeasurementBoundary::for_class(MeasurementClass::FacadeRunEndToEnd),
    )
}

pub(super) fn failure_receipt(
    context: FailureContext,
    backend: BackendKind,
    cleanup: CleanupEvidence,
) -> ExecutionReceipt {
    ExecutionReceipt::new(
        OperationKind::Run,
        context.operation_id,
        context.instance_id,
        context.machine_name,
        context.fingerprint,
        context.workload,
        backend,
        Observation::Unavailable(ObservationUnavailable::NotReached),
        Observation::Unavailable(ObservationUnavailable::NotReached),
        Observation::Unavailable(ObservationUnavailable::NotReached),
        context.requested_shape,
        EffectiveShape::unavailable(ObservationUnavailable::NotReached),
        EffectiveNetwork::unavailable(ObservationUnavailable::NotReached),
        context.milestones,
        TerminalStatus::Failed,
        Observation::Unavailable(ObservationUnavailable::NotReached),
        cleanup,
        MeasurementBoundary::for_class(MeasurementClass::FacadeRunEndToEnd),
    )
}

pub(super) fn failure_receipt_after_launch(
    context: FailureContext,
    launch: LaunchEvidence,
    cleanup: CleanupEvidence,
) -> ExecutionReceipt {
    ExecutionReceipt::new(
        OperationKind::Run,
        context.operation_id,
        context.instance_id,
        context.machine_name,
        context.fingerprint,
        context.workload,
        launch.backend,
        Observation::Observed(launch.isolation),
        Observation::Observed(launch.preparation),
        Observation::Observed(launch.digest_binding),
        context.requested_shape,
        launch.effective_shape,
        launch.effective_network,
        context.milestones,
        TerminalStatus::Failed,
        Observation::Unavailable(ObservationUnavailable::NotReached),
        cleanup,
        MeasurementBoundary::for_class(MeasurementClass::FacadeRunEndToEnd),
    )
}

pub(super) struct CleanupResult {
    pub(super) complete: bool,
    pub(super) evidence: CleanupEvidence,
    pub(super) failure_kind: Option<RunFailureKind>,
}

impl CleanupResult {
    pub(super) const fn invalid() -> Self {
        Self {
            complete: false,
            evidence: CleanupEvidence::incomplete_owned_machine(),
            failure_kind: Some(ObservationMismatch),
        }
    }
}
