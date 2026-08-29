use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{
    BackendKind, CleanupEvidence, DigestBinding, EffectiveNetwork, EffectiveShape, EvidenceClass,
    InstanceId, IsolationClass, MachineName, MachineShape, MeasurementBoundary, Milestone,
    Observation, OperationId, OperationKind, OutputMetadata, PreparationClass, RequestFingerprint,
    TerminalStatus, WorkloadEvidence,
};

mod validation;

const SCHEMA_VERSION: u16 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExecutionReceipt {
    schema_version: u16,
    soma_version: String,
    evidence_class: EvidenceClass,
    operation: OperationKind,
    operation_id: OperationId,
    instance_id: InstanceId,
    machine_name: Option<MachineName>,
    request_fingerprint: RequestFingerprint,
    workload: WorkloadEvidence,
    backend: BackendKind,
    isolation: Observation<IsolationClass>,
    preparation: Observation<PreparationClass>,
    digest_binding: Observation<DigestBinding>,
    requested_shape: MachineShape,
    effective_shape: EffectiveShape,
    effective_network: EffectiveNetwork,
    milestones: Vec<Milestone>,
    terminal_status: TerminalStatus,
    output: Observation<OutputMetadata>,
    cleanup: CleanupEvidence,
    measurement: MeasurementBoundary,
}

impl ExecutionReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        operation: OperationKind,
        operation_id: OperationId,
        instance_id: InstanceId,
        machine_name: Option<MachineName>,
        request_fingerprint: RequestFingerprint,
        workload: WorkloadEvidence,
        backend: BackendKind,
        isolation: Observation<IsolationClass>,
        preparation: Observation<PreparationClass>,
        digest_binding: Observation<DigestBinding>,
        requested_shape: MachineShape,
        effective_shape: EffectiveShape,
        effective_network: EffectiveNetwork,
        milestones: Vec<Milestone>,
        terminal_status: TerminalStatus,
        output: Observation<OutputMetadata>,
        cleanup: CleanupEvidence,
        measurement: MeasurementBoundary,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            soma_version: env!("CARGO_PKG_VERSION").to_owned(),
            evidence_class: EvidenceClass::BasicBackendReported,
            operation,
            operation_id,
            instance_id,
            machine_name,
            request_fingerprint,
            workload,
            backend,
            isolation,
            preparation,
            digest_binding,
            requested_shape,
            effective_shape,
            effective_network,
            milestones,
            terminal_status,
            output,
            cleanup,
            measurement,
        }
    }

    #[must_use]
    pub const fn terminal_status(&self) -> &TerminalStatus {
        &self.terminal_status
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub fn soma_version(&self) -> &str {
        &self.soma_version
    }

    #[must_use]
    pub const fn evidence_class(&self) -> EvidenceClass {
        self.evidence_class
    }

    #[must_use]
    pub const fn operation(&self) -> OperationKind {
        self.operation
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn workload(&self) -> &WorkloadEvidence {
        &self.workload
    }

    #[must_use]
    pub const fn backend(&self) -> BackendKind {
        self.backend
    }

    #[must_use]
    pub const fn isolation(&self) -> &Observation<IsolationClass> {
        &self.isolation
    }

    #[must_use]
    pub const fn preparation(&self) -> &Observation<PreparationClass> {
        &self.preparation
    }

    #[must_use]
    pub const fn effective_shape(&self) -> &EffectiveShape {
        &self.effective_shape
    }

    #[must_use]
    pub const fn effective_network(&self) -> &EffectiveNetwork {
        &self.effective_network
    }

    #[must_use]
    pub const fn requested_shape(&self) -> &MachineShape {
        &self.requested_shape
    }

    #[must_use]
    pub const fn cleanup(&self) -> &CleanupEvidence {
        &self.cleanup
    }

    #[must_use]
    pub fn milestones(&self) -> &[Milestone] {
        &self.milestones
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &RequestFingerprint {
        &self.request_fingerprint
    }

    #[must_use]
    pub const fn machine_name(&self) -> Option<&MachineName> {
        self.machine_name.as_ref()
    }

    #[must_use]
    pub const fn digest_binding(&self) -> &Observation<DigestBinding> {
        &self.digest_binding
    }

    #[must_use]
    pub const fn output(&self) -> &Observation<OutputMetadata> {
        &self.output
    }

    #[must_use]
    pub const fn measurement(&self) -> &MeasurementBoundary {
        &self.measurement
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptWire {
    schema_version: u16,
    soma_version: String,
    evidence_class: EvidenceClass,
    operation: OperationKind,
    operation_id: OperationId,
    instance_id: InstanceId,
    machine_name: Option<MachineName>,
    request_fingerprint: RequestFingerprint,
    workload: WorkloadEvidence,
    backend: BackendKind,
    isolation: Observation<IsolationClass>,
    preparation: Observation<PreparationClass>,
    digest_binding: Observation<DigestBinding>,
    requested_shape: MachineShape,
    effective_shape: EffectiveShape,
    effective_network: EffectiveNetwork,
    milestones: Vec<Milestone>,
    terminal_status: TerminalStatus,
    output: Observation<OutputMetadata>,
    cleanup: CleanupEvidence,
    measurement: MeasurementBoundary,
}

impl From<ReceiptWire> for ExecutionReceipt {
    fn from(wire: ReceiptWire) -> Self {
        Self {
            schema_version: wire.schema_version,
            soma_version: wire.soma_version,
            evidence_class: wire.evidence_class,
            operation: wire.operation,
            operation_id: wire.operation_id,
            instance_id: wire.instance_id,
            machine_name: wire.machine_name,
            request_fingerprint: wire.request_fingerprint,
            workload: wire.workload,
            backend: wire.backend,
            isolation: wire.isolation,
            preparation: wire.preparation,
            digest_binding: wire.digest_binding,
            requested_shape: wire.requested_shape,
            effective_shape: wire.effective_shape,
            effective_network: wire.effective_network,
            milestones: wire.milestones,
            terminal_status: wire.terminal_status,
            output: wire.output,
            cleanup: wire.cleanup,
            measurement: wire.measurement,
        }
    }
}

impl<'de> Deserialize<'de> for ExecutionReceipt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let receipt = Self::from(ReceiptWire::deserialize(deserializer)?);
        receipt.validate().map_err(D::Error::custom)?;
        Ok(receipt)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiptValidationError;

impl fmt::Display for ReceiptValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("execution receipt failed schema validation")
    }
}

impl std::error::Error for ReceiptValidationError {}
