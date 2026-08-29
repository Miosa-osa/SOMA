use soma::{BackendFailure, BackendFailureKind, EffectivePortPublication, OperationId};
use soma_macos::{
    BackendError, CommandFailureReason, ExecutionStatus, InstanceId, ProcessFailureKind,
    PublishedPort,
};

use super::{adapter::MacBackend, config::control_limits, network::verify_released};

impl MacBackend {
    pub(super) fn rollback(
        &mut self,
        operation: &OperationId,
        key: &str,
        instance: InstanceId,
        publications: &[EffectivePortPublication],
        primary: &BackendError,
    ) -> BackendFailure {
        let kind = backend_failure_kind(primary);
        self.rollback_kind(operation, key, instance, publications, kind)
    }

    pub(super) fn rollback_kind(
        &mut self,
        operation: &OperationId,
        key: &str,
        instance: InstanceId,
        publications: &[EffectivePortPublication],
        kind: BackendFailureKind,
    ) -> BackendFailure {
        match self.backend.delete(instance, control_limits()) {
            Ok(_) if verify_released(publications).is_ok() => {
                self.already_cleaned.insert(key.to_owned());
                self.failure(operation, kind)
            }
            Ok(_) | Err(_) => self.failure(operation, BackendFailureKind::CleanupFailure),
        }
    }

    pub(super) fn map_error(
        &mut self,
        operation: &OperationId,
        error: &BackendError,
    ) -> BackendFailure {
        self.failure(operation, backend_failure_kind(error))
    }

    pub(super) fn failure(
        &mut self,
        operation: &OperationId,
        kind: BackendFailureKind,
    ) -> BackendFailure {
        BackendFailure::new(kind, self.clocks.elapsed_ns(operation))
    }
}

pub(super) fn create_failure_proved_cleanup(error: &BackendError) -> bool {
    matches!(error, BackendError::Command { .. })
}

pub(super) fn error_proved_invalidation(error: &BackendError) -> bool {
    matches!(error, BackendError::ManagedExecutionInvalidated { .. })
}

pub(super) fn invalidation_cleanup_ports(error: &BackendError) -> Option<&[PublishedPort]> {
    match error {
        BackendError::ManagedExecutionInvalidated {
            cleanup_published_ports,
            ..
        } => cleanup_published_ports.as_deref(),
        _ => None,
    }
}

const fn backend_failure_kind(error: &BackendError) -> BackendFailureKind {
    match error {
        BackendError::UnsupportedHost | BackendError::UnsupportedVersion { .. } => {
            BackendFailureKind::Unsupported
        }
        BackendError::ImageResolution { .. } => BackendFailureKind::WorkloadRejected,
        BackendError::CleanupFailed { .. } => BackendFailureKind::CleanupFailure,
        BackendError::ManagedExecutionInvalidated { failure, .. }
        | BackendError::Command { failure } => command_failure_kind(failure.reason()),
    }
}

const fn command_failure_kind(reason: CommandFailureReason) -> BackendFailureKind {
    match reason {
        CommandFailureReason::Status(ExecutionStatus::TimedOut) => BackendFailureKind::Timeout,
        CommandFailureReason::Status(ExecutionStatus::OutputLimitExceeded) => {
            BackendFailureKind::OutputLimit
        }
        CommandFailureReason::Process(
            ProcessFailureKind::ExecutableUnavailable
            | ProcessFailureKind::PermissionDenied
            | ProcessFailureKind::SpawnFailed,
        ) => BackendFailureKind::Unavailable,
        CommandFailureReason::Process(_)
        | CommandFailureReason::Status(_)
        | CommandFailureReason::Ownership(_)
        | CommandFailureReason::InvalidJson
        | CommandFailureReason::MissingVersionComponent
        | CommandFailureReason::RuntimeNotRunning => BackendFailureKind::IsolationFailure,
    }
}
