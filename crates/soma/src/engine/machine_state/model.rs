use serde::{Deserialize, Serialize};

use crate::{
    BackendKind, ExecutionReceipt, InstanceId, MachineName, MachineShape, OperationId,
    OperationKind, RequestFingerprint, StateRevision, TerminalStatus, WorkloadIdentity,
};

pub(super) const STATE_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Serialize)]
pub(in crate::engine) struct DurableMachine {
    pub(super) schema_version: u16,
    pub(in crate::engine) instance_id: InstanceId,
    pub(in crate::engine) phase: DurablePhase,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(in crate::engine) enum DurablePhase {
    Launching {
        intent: Box<LaunchIntent>,
    },
    Active {
        active: Box<ActiveMachine>,
    },
    Executing {
        active: Box<ActiveMachine>,
        operation_id: OperationId,
        request_fingerprint: RequestFingerprint,
    },
    Terminating {
        active: Box<ActiveMachine>,
        operation: OperationKind,
        operation_id: OperationId,
        request_fingerprint: RequestFingerprint,
    },
    Terminal {
        basis: Box<TerminalBasis>,
        operation: OperationKind,
        operation_id: OperationId,
        request_fingerprint: RequestFingerprint,
        receipt: Box<ExecutionReceipt>,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "basis", rename_all = "snake_case", deny_unknown_fields)]
pub(in crate::engine) enum TerminalBasis {
    Launch { intent: Box<LaunchIntent> },
    Active { active: Box<ActiveMachine> },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::engine) struct LaunchIntent {
    pub(in crate::engine) operation_id: OperationId,
    pub(in crate::engine) machine_name: Option<MachineName>,
    pub(in crate::engine) workload: WorkloadIdentity,
    pub(in crate::engine) requested_shape: MachineShape,
    pub(in crate::engine) backend: BackendKind,
    pub(in crate::engine) request_fingerprint: RequestFingerprint,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::engine) struct ActiveMachine {
    pub(in crate::engine) launch_receipt: ExecutionReceipt,
    #[serde(default)]
    pub(in crate::engine) completed_executions: Vec<ExecutionTombstone>,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::engine) struct ExecutionTombstone {
    pub(in crate::engine) operation_id: OperationId,
    pub(in crate::engine) request_fingerprint: RequestFingerprint,
    pub(in crate::engine) terminal_status: TerminalStatus,
    pub(in crate::engine) receipt_digest: RequestFingerprint,
}

pub(in crate::engine) struct VersionedMachine {
    pub(in crate::engine) revision: StateRevision,
    pub(in crate::engine) machine: DurableMachine,
}
