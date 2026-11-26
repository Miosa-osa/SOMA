//! Admit, launch and connect one jailed worker.

use super::{
    ATTESTATION_CEILING, ActivationChannel, Anchors, BackendFailureKind, ContractVersion,
    DeclaredDevices, DiskBytes, Generation, GenerationId, HotplugBytes, InstanceId, JailHandle,
    Jailed, LAUNCH_CEILING, Launch, Launching, MIB, MachineSpec, MemoryBytes, NetworkLaunch,
    OperationId, Outcome, ProbeReport, Request, VcpuCount, VmmInstanceId, build, fresh16,
    instance_bytes, outcome,
};
use std::{
    fs::File,
    os::fd::AsFd as _,
    time::{Duration, Instant},
};

impl Jailed {
    /// Reads and checks the worker's own attestation before anything is asked of it.
    ///
    /// The worker already refuses to serve an attestation that does not describe a jail. This
    /// side checks the same report for the same reason from the other end: a broker that
    /// accepted a worker it could not see the containment of would be trusting the process it
    /// exists to constrain.
    pub(super) fn admit(
        &mut self,
        anchors: &Anchors,
        instance: &InstanceId,
    ) -> Result<ProbeReport, BackendFailureKind> {
        let text = self.control.receive(ATTESTATION_CEILING)?;
        let pid = self.handle.as_ref().map_or(0, JailHandle::pid);
        anchors.record(instance.as_str(), &format!("jailed pid={pid} {text}"));
        let report =
            ProbeReport::decode(&text).map_err(|_| BackendFailureKind::IsolationFailure)?;
        if outcome::describes_a_jail(&report) {
            Ok(report)
        } else {
            Err(BackendFailureKind::IsolationFailure)
        }
    }

    /// Launches the machine and then narrows the worker's filter to its steady state.
    ///
    /// The order is the whole point of the two phases. Restoring a machine creates event
    /// descriptors, epoll sets, and threads, which the startup filter admits; serving commands
    /// needs none of them, so the filter narrows the moment the machine exists and every
    /// startup-only syscall becomes a kill for the rest of the worker's life.
    pub(super) fn perform_launch(
        &mut self,
        anchors: &Anchors,
        launching: &Launching<'_>,
        devices: DeclaredDevices,
        recovery: Option<build::Recovery>,
        activation: Option<(ActivationChannel, NetworkLaunch<'_>)>,
    ) -> Result<(), BackendFailureKind> {
        let mut machine = MachineSpec::new(
            VcpuCount::new(launching.prepared.manifest.shape.vcpu_count)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
            MemoryBytes::new(launching.memory_mib.saturating_mul(MIB))
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
            DiskBytes::new(launching.disk_mib.saturating_mul(MIB).max(MIB))
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
        )
        .with_contract(
            ContractVersion::new(launching.prepared.manifest.machine_contract.version)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
        );
        // The region is declared by the Generation, so a launch carries it only when it did.
        if let Some(bytes) = super::super::hotplug::declared_bytes(launching.prepared) {
            machine = machine.with_hotplug(
                HotplugBytes::new(bytes).map_err(|_| BackendFailureKind::WorkloadRejected)?,
            );
        }
        let operation = OperationId::new(
            activation
                .as_ref()
                .map_or_else(fresh16, |(_, network)| network.operation),
        )
        .map_err(|_| BackendFailureKind::WorkloadRejected)?;
        let launch = Launch::new(
            operation,
            self.instance,
            Generation::new(
                GenerationId::new(launching.generation_bytes)
                    .map_err(|_| BackendFailureKind::WorkloadRejected)?,
                machine,
                devices,
            ),
        );
        let instance = launching.instance.as_str();
        let reply = match recovery {
            None => match activation {
                Some((channel, mut network)) => self.connected_launch(
                    &Request::Launch(launch),
                    None,
                    &channel,
                    &mut network,
                    LAUNCH_CEILING,
                ),
                None => self.control.ask(&Request::Launch(launch), LAUNCH_CEILING),
            },
            Some(recovery) => {
                let digest = *recovery.digest.as_bytes();
                let from = launching.checkpoint.map(|(source, _)| source);
                let request = match from {
                    Some(source) if source != launching.instance => {
                        let source = VmmInstanceId::new(instance_bytes(source)?)
                            .map_err(|_| BackendFailureKind::WorkloadRejected)?;
                        Request::Fork(launch, digest, source)
                    }
                    _ => Request::Recover(launch, digest),
                };
                self.offer_attestations(anchors, instance, &recovery.attestations)?;
                let budget = super::super::checkpoint::CHECKPOINT_CEILING + LAUNCH_CEILING;
                let overlay = recovery.overlay.as_ref();
                match (activation, overlay) {
                    (Some((channel, mut network)), overlay) => {
                        self.connected_launch(&request, overlay, &channel, &mut network, budget)
                    }
                    (None, Some(overlay)) => {
                        self.control
                            .ask_with_files(&request, &[overlay.as_fd()], budget)
                    }
                    (None, None) => self.control.ask(&request, budget),
                }
            }
        }
        .inspect_err(|error| {
            anchors.record(
                instance,
                &format!("jailed launch exchange failed: {error:?}"),
            );
        })?;
        if let soma_vmm::control::Outcome::Failure { detail, .. } = &reply {
            anchors.record(instance, &format!("jailed worker refused launch: {detail}"));
        }
        outcome::ready(&reply)?;
        if !matches!(reply, soma_vmm::control::Outcome::Ready { operation_id } if operation_id == operation)
        {
            return Err(BackendFailureKind::GuestFailure);
        }
        self.control
            .ask(&Request::Seal, ATTESTATION_CEILING)
            .and_then(|reply| outcome::sealed(&reply))
            .inspect_err(|error| {
                anchors.record(instance, &format!("jailed worker did not seal: {error:?}"));
            })
    }

    /// Hands the worker the broker's records of the immutable restore inputs before recovery.
    ///
    /// A worker that predates a record rejects only that packet, and its recovery then hashes
    /// the artifact as before; only a broken control channel fails the launch.
    fn offer_attestations(
        &self,
        anchors: &Anchors,
        instance: &str,
        attestations: &soma_kvm::snapshot::RestoreAttestations,
    ) -> Result<(), BackendFailureKind> {
        use soma_kvm::snapshot::AttestedArtifact;
        for (kind, record) in [
            (AttestedArtifact::Memory, attestations.memory),
            (AttestedArtifact::Root, attestations.root),
            (AttestedArtifact::Head, attestations.head),
        ] {
            let Some(record) = record else { continue };
            let reply = self.control.ask(
                &Request::AttestArtifact((kind, record)),
                ATTESTATION_CEILING,
            )?;
            if !matches!(reply, Outcome::ArtifactAttested) {
                anchors.record(
                    instance,
                    &format!("worker declined an attestation: {reply:?}"),
                );
            }
        }
        Ok(())
    }

    pub(super) fn connected_launch(
        &self,
        request: &Request,
        overlay: Option<&File>,
        channel: &ActivationChannel,
        network: &mut NetworkLaunch<'_>,
        budget: Duration,
    ) -> Result<Outcome, BackendFailureKind> {
        let began = Instant::now();
        channel
            .offer(network.launch, &network.pending)
            .map_err(|_| BackendFailureKind::GuestFailure)?;
        if let Some(overlay) = overlay {
            self.control.tell_with_files(request, &[overlay.as_fd()])?;
        } else {
            self.control.tell(request)?;
        }
        channel
            .confirm(budget.saturating_sub(began.elapsed()), |receipt| {
                network.egress.activate_sealed(receipt)
            })
            .map_err(|_| BackendFailureKind::GuestFailure)?;
        let text = self
            .control
            .receive(budget.saturating_sub(began.elapsed()))?;
        Outcome::decode(&text).map_err(|_| BackendFailureKind::Unavailable)
    }

    /// Whether this machine may still be addressed.
    pub(in super::super) const fn is_usable(&self) -> bool {
        !self.poisoned
    }
}
