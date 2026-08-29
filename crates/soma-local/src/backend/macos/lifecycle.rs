use soma::{
    BackendFailure, BackendFailureKind, BackendKind, CleanupDisposition, CleanupEvidence,
    CleanupMethod, CleanupObservation, CleanupReason, CleanupRequest, CleanupTimes, DigestBinding,
    InspectionObservation, InspectionRequest, IsolationClass, LaunchObservation, LaunchRequest,
    LaunchTimes, NetworkCleanupEvidence, PreparationClass,
};
use soma_macos::{CreateMachine, GuestCommand, MachineShape, StopOptions};

use super::{
    adapter::{MacBackend, MacPreparedWorkload},
    config::{STOP_GRACE_SECONDS, control_limits, mac_instance},
    evidence::{effective_network, inspection_state, launch_evidence},
    failure::create_failure_proved_cleanup,
    network::{configured_publications, prepare, verify_active, verify_released},
};

impl MacBackend {
    pub(in crate::backend) fn launch(
        &mut self,
        request: &LaunchRequest<'_, MacPreparedWorkload>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let operation = request.operation_id();
        let admitted = self.clocks.elapsed_ns(operation);
        if request.prepared().identity != *request.workload() {
            return Err(self.failure(operation, BackendFailureKind::WorkloadRejected));
        }
        let instance = mac_instance(request.instance_id().as_str())
            .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let memory_bytes = request
            .shape()
            .memory_mib()
            .checked_mul(1_048_576)
            .ok_or_else(|| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let shape = MachineShape::new(request.shape().vcpu_count(), memory_bytes)
            .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let keeper = GuestCommand::new("/bin/sleep", ["infinity"])
            .expect("the internal keeper is a valid bounded direct command");
        let prepared_network = prepare(request.shape().capabilities().network_policy())
            .map_err(|kind| self.failure(operation, kind))?;
        let create = CreateMachine::new(
            instance.clone(),
            request.prepared().image.clone(),
            shape,
            keeper,
            control_limits(),
        )
        .with_network(prepared_network.configuration().clone());
        if let Err(error) = self.backend.create(&create) {
            if create_failure_proved_cleanup(&error) {
                self.already_cleaned
                    .insert(request.instance_id().as_str().to_owned());
            }
            return Err(self.map_error(operation, &error));
        }
        let network_expectation = prepared_network.begin_activation();
        if let Err(error) = self.backend.start(instance.clone(), control_limits()) {
            return Err(self.rollback(
                operation,
                request.instance_id().as_str(),
                instance,
                network_expectation.publications(),
                &error,
            ));
        }
        let launched = self.clocks.elapsed_ns(operation);
        let inspection = match self.backend.inspect(instance.clone(), control_limits()) {
            Ok(inspection) => inspection,
            Err(error) => {
                return Err(self.rollback(
                    operation,
                    request.instance_id().as_str(),
                    instance,
                    network_expectation.publications(),
                    &error,
                ));
            }
        };
        if let Err(kind) = verify_active(&network_expectation, &inspection) {
            return Err(self.rollback_kind(
                operation,
                request.instance_id().as_str(),
                instance,
                network_expectation.publications(),
                kind,
            ));
        }
        let (effective_shape, effective_network) =
            match launch_evidence(request.shape(), &inspection, &network_expectation) {
                Ok(evidence) => evidence,
                Err(kind) => {
                    return Err(self.rollback_kind(
                        operation,
                        request.instance_id().as_str(),
                        instance,
                        network_expectation.publications(),
                        kind,
                    ));
                }
            };
        let ready = self.clocks.elapsed_ns(operation);
        Ok(LaunchObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            BackendKind::MacosVirtualization,
            IsolationClass::HardwareVirtualMachine,
            PreparationClass::OnDemand,
            DigestBinding::ObservedOnly,
            effective_shape,
            effective_network,
            LaunchTimes::new(admitted, launched, ready),
        ))
    }

    pub(in crate::backend) fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        let operation = request.operation_id();
        self.clocks.elapsed_ns(operation);
        let instance = mac_instance(request.instance_id().as_str())
            .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let inspection = self
            .backend
            .inspect(instance, control_limits())
            .map_err(|error| self.map_error(operation, &error))?;
        let state = inspection_state(&inspection)
            .ok_or_else(|| self.failure(operation, BackendFailureKind::IsolationFailure))?;
        let publications =
            configured_publications(&inspection).map_err(|kind| self.failure(operation, kind))?;
        let network_expectation = super::network::ActivationExpectation::observed(publications);
        verify_active(&network_expectation, &inspection)
            .map_err(|kind| self.failure(operation, kind))?;
        let network = effective_network(
            request.shape().capabilities().network_policy(),
            &inspection,
            &network_expectation,
        )
        .map_err(|kind| self.failure(operation, kind))?;
        Ok(InspectionObservation::observed(
            request,
            BackendKind::MacosVirtualization,
            state,
            network,
            self.clocks.elapsed_ns(operation),
        ))
    }

    pub(in crate::backend) fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let key = request.instance_id().as_str();
        let method = if self.already_cleaned.remove(key) {
            CleanupMethod::Forced
        } else {
            let instance = mac_instance(key)
                .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
            self.release(operation, instance, request.reason())?
        };
        let network = NetworkCleanupEvidence::uniform(CleanupDisposition::Complete)
            .with_lease(CleanupDisposition::NotOwned)
            .with_proxy_policy(CleanupDisposition::NotOwned);
        let evidence = CleanupEvidence::complete_owned_machine()
            .with_network(network)
            .with_method(method);
        Ok(CleanupObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            evidence,
            CleanupTimes::new(started, self.clocks.elapsed_ns(operation)),
        ))
    }

    fn release(
        &mut self,
        operation: &soma::OperationId,
        instance: soma_macos::InstanceId,
        reason: CleanupReason,
    ) -> Result<CleanupMethod, BackendFailure> {
        let inspection = self
            .backend
            .inspect(instance.clone(), control_limits())
            .map_err(|error| self.map_error(operation, &error))?;
        let publications =
            configured_publications(&inspection).map_err(|kind| self.failure(operation, kind))?;
        if reason == CleanupReason::GracefulStop {
            let stopped = self
                .backend
                .stop(
                    instance.clone(),
                    StopOptions::new(STOP_GRACE_SECONDS, control_limits()),
                )
                .is_ok();
            self.backend
                .delete(instance, control_limits())
                .map_err(|error| self.map_error(operation, &error))?;
            verify_released(&publications).map_err(|kind| self.failure(operation, kind))?;
            return Ok(if stopped {
                CleanupMethod::Graceful
            } else {
                CleanupMethod::GracefulThenForced
            });
        }
        self.backend
            .delete(instance, control_limits())
            .map_err(|error| self.map_error(operation, &error))?;
        verify_released(&publications).map_err(|kind| self.failure(operation, kind))?;
        Ok(CleanupMethod::Forced)
    }
}
