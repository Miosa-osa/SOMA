use soma::{
    BackendFailure, BackendFailureKind, BackendKind, DigestBinding, DnsPolicy, EffectiveShape,
    EgressPolicy, IsolationClass, LaunchObservation, LaunchRequest, LaunchTimes, Observation,
    ObservationUnavailable, PreparationClass,
};

use super::command::{CONTROL_TIMEOUT, command, command_owned};
use super::container::{container_name, remove};
use super::network::effective_network;
use super::resolve::DockerPreparedWorkload;
use super::{DockerBackend, failure};

impl DockerBackend {
    pub(in crate::backend) fn launch(
        &mut self,
        request: &LaunchRequest<'_, Box<dyn std::any::Any + Send>>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let operation = request.operation_id();
        let admitted = self.clocks.elapsed_ns(operation);
        let prepared = request
            .prepared()
            .downcast_ref::<DockerPreparedWorkload>()
            .ok_or_else(|| failure(operation, BackendFailureKind::WorkloadRejected))?;
        if prepared.identity != *request.workload() {
            return Err(failure(operation, BackendFailureKind::WorkloadRejected));
        }
        let name = container_name(request.instance_id().as_str());
        let network = request.shape().capabilities().network_policy();
        let network_mode = match (network.egress(), network.dns(), network.published_ports()) {
            (EgressPolicy::Denied, DnsPolicy::Denied, []) => "none",
            (EgressPolicy::Unrestricted, DnsPolicy::System, []) => "bridge",
            _ => return Err(failure(operation, BackendFailureKind::Unsupported)),
        };
        let memory = format!("{}m", request.shape().memory_mib());
        let cpus = request.shape().vcpu_count().to_string();
        let args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            name.clone(),
            "--label".to_owned(),
            format!("com.miosa.soma.instance={}", request.instance_id().as_str()),
            "--cpus".to_owned(),
            cpus,
            "--memory".to_owned(),
            memory,
            "--pids-limit".to_owned(),
            "256".to_owned(),
            "--read-only".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,nosuid,nodev,size=64m".to_owned(),
            "--network".to_owned(),
            network_mode.to_owned(),
            "--entrypoint".to_owned(),
            "/bin/sh".to_owned(),
            prepared.image.clone(),
            "-c".to_owned(),
            "while :; do sleep 3600; done".to_owned(),
        ];
        let created = command_owned(&args, CONTROL_TIMEOUT);
        if !created.status.is_some_and(|status| status.success()) {
            return Err(failure(operation, BackendFailureKind::GuestFailure));
        }
        let started = command(&["start", &name], CONTROL_TIMEOUT);
        if !started.status.is_some_and(|status| status.success()) {
            let _ = remove(&name);
            return Err(failure(operation, BackendFailureKind::GuestFailure));
        }
        let launched = self.clocks.elapsed_ns(operation);
        let ready = self.clocks.elapsed_ns(operation);
        let shape = EffectiveShape::new(
            Observation::Observed(request.shape().vcpu_count()),
            Observation::Observed(request.shape().memory_mib()),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
        )
        .map_err(|_| failure(operation, BackendFailureKind::IsolationFailure))?;
        Ok(LaunchObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            BackendKind::DockerContainer,
            IsolationClass::LinuxContainer,
            PreparationClass::OnDemand,
            DigestBinding::ObservedOnly,
            shape,
            effective_network(network_mode),
            LaunchTimes::new(admitted, launched, ready),
        ))
    }
}
