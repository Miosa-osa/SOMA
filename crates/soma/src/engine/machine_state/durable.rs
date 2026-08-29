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
            },
        }
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
            DurablePhase::Launching { .. } => Ok(()),
            DurablePhase::Active { active } | DurablePhase::Executing { active, .. } => {
                active.validate(&self.instance_id)
            }
            DurablePhase::Terminating {
                active, operation, ..
            } => {
                if !matches!(operation, OperationKind::Stop | OperationKind::Destroy) {
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
                            OperationKind::Stop | OperationKind::Destroy | OperationKind::Execute
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
