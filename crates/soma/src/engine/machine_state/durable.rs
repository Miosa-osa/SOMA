use serde::{Deserialize, Deserializer, de::Error as _};

use crate::{
    InstanceId, OperationKind, StateRecord, StateStoreFailure, StateStoreFailureKind, StoredState,
};

use super::{
    ActiveMachine, DurableMachine, DurablePhase, LaunchIntent, TerminalBasis, VersionedMachine,
    corrupt, model::STATE_SCHEMA_VERSION,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableMachineWire {
    schema_version: u16,
    instance_id: InstanceId,
    phase: DurablePhase,
}

impl<'de> Deserialize<'de> for DurableMachine {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = DurableMachineWire::deserialize(deserializer)?;
        if wire.schema_version != STATE_SCHEMA_VERSION {
            return Err(D::Error::custom("unsupported machine state schema"));
        }
        let machine = Self {
            schema_version: wire.schema_version,
            instance_id: wire.instance_id,
            phase: wire.phase,
        };
        machine
            .validate()
            .map_err(|_| D::Error::custom("invalid durable machine state"))?;
        Ok(machine)
    }
}

impl DurableMachine {
    pub(in crate::engine) fn launching(instance_id: InstanceId, intent: LaunchIntent) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            instance_id,
            phase: DurablePhase::Launching {
                intent: Box::new(intent),
                prior_stop: None,
            },
        }
    }

    pub(in crate::engine) fn restoring(
        instance: InstanceId,
        intent: LaunchIntent,
        stopped: Self,
    ) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            instance_id: instance,
            phase: DurablePhase::Launching {
                intent: Box::new(intent),
                prior_stop: Some(Box::new(stopped)),
            },
        }
    }

    pub(in crate::engine) fn stopped_source(
        &self,
        intent: &LaunchIntent,
    ) -> Option<&ActiveMachine> {
        let DurablePhase::Terminal {
            basis,
            operation: OperationKind::Stop,
            operation_id,
            receipt,
            ..
        } = &self.phase
        else {
            return None;
        };
        let TerminalBasis::Active { active } = basis.as_ref() else {
            return None;
        };
        (receipt.terminal_status() == &crate::TerminalStatus::Stopped
            && operation_id != &intent.operation_id
            && active.launch_receipt.operation_id() != &intent.operation_id
            && active.completed(&intent.operation_id).is_none()
            && active.workload() == Some(&intent.workload)
            && active.backend() == intent.backend
            && active.machine_name() == intent.machine_name.as_ref()
            && active.shape() == &intent.requested_shape)
            .then_some(active)
    }

    pub(in crate::engine) fn active(
        instance_id: InstanceId,
        launch_receipt: crate::ExecutionReceipt,
    ) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            instance_id,
            phase: DurablePhase::Active {
                active: Box::new(ActiveMachine {
                    launch_receipt,
                    completed_executions: Vec::new(),
                }),
            },
        }
    }

    pub(in crate::engine) fn encode(&self) -> Result<StateRecord, StateStoreFailure> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|_| StateStoreFailure::new(StateStoreFailureKind::Corrupt))?;
        StateRecord::from_bytes(bytes)
    }

    pub(in crate::engine) fn decode(
        stored: &StoredState,
        expected_instance: &InstanceId,
    ) -> Result<VersionedMachine, StateStoreFailure> {
        let schema = serde_json::from_slice::<SchemaProbe>(stored.record().as_bytes())
            .map_err(|_| StateStoreFailure::new(StateStoreFailureKind::Corrupt))?;
        if schema.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateStoreFailure::new(
                StateStoreFailureKind::UnsupportedVersion,
            ));
        }
        let machine = serde_json::from_slice::<Self>(stored.record().as_bytes())
            .map_err(|_| StateStoreFailure::new(StateStoreFailureKind::Corrupt))?;
        if machine.instance_id != *expected_instance {
            return Err(StateStoreFailure::new(StateStoreFailureKind::Corrupt));
        }
        Ok(VersionedMachine {
            revision: stored.revision(),
            machine,
        })
    }

    fn validate(&self) -> Result<(), StateStoreFailure> {
        match &self.phase {
            DurablePhase::Launching { intent, prior_stop } => {
                if let Some(stopped) = prior_stop
                    && (stopped.instance_id != self.instance_id
                        || stopped.stopped_source(intent).is_none())
                {
                    return Err(corrupt());
                }
                Ok(())
            }
            DurablePhase::Active { active } | DurablePhase::Executing { active, .. } => {
                active.validate(&self.instance_id)
            }
            DurablePhase::Terminating {
                active, operation, ..
            } => {
                if !matches!(
                    operation,
                    OperationKind::Stop | OperationKind::Destroy | OperationKind::Suspend
                ) {
                    return Err(corrupt());
                }
                active.validate(&self.instance_id)
            }
            DurablePhase::Terminal {
                basis,
                operation,
                operation_id,
                request_fingerprint,
                receipt,
            } => {
                basis.validate(&self.instance_id)?;
                if !matches!(
                    (basis.as_ref(), operation),
                    (TerminalBasis::Launch { .. }, OperationKind::Launch)
                        | (
                            TerminalBasis::Active { .. },
                            OperationKind::Stop
                                | OperationKind::Destroy
                                | OperationKind::Suspend
                                | OperationKind::Execute
                        )
                ) || receipt.operation() != *operation
                    || receipt.operation_id() != operation_id
                    || receipt.instance_id() != &self.instance_id
                    || receipt.request_fingerprint() != request_fingerprint
                {
                    return Err(corrupt());
                }
                Ok(())
            }
        }
    }
}

impl TerminalBasis {
    fn validate(&self, instance_id: &InstanceId) -> Result<(), StateStoreFailure> {
        match self {
            Self::Launch { .. } => Ok(()),
            Self::Active { active } => active.validate(instance_id),
        }
    }
}

#[derive(Deserialize)]
struct SchemaProbe {
    schema_version: u16,
}
