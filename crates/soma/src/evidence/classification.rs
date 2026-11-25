use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    LinuxKvm,
    MacosVirtualization,
    DockerContainer,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationClass {
    HardwareVirtualMachine,
    LinuxContainer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationClass {
    OnDemand,
    PreparedWorker,
    PausedLease,
    ReadyLease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestBinding {
    ObservedOnly,
    LaunchEnforced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    BasicBackendReported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Run,
    Launch,
    Execute,
    Stop,
    Inspect,
    Destroy,
    /// The Instance was read into a Suspension and ended (ADR 0034).
    Suspend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    Exited {
        code: i32,
    },
    Signaled {
        signal: Option<i32>,
    },
    TimedOut,
    OutputLimitExceeded,
    Failed,
    Ready,
    Stopped,
    Inspected {
        state: MachineState,
    },
    Destroyed,
    /// The Instance was read into a Suspension; a resume is a new Instance.
    Suspended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineState {
    Ready,
    Paused,
    Stopping,
    /// The machine is gone: its record says running, but the backend reports its host absent, as
    /// after its worker crashed or was killed. Only an inspection reports it; the record is left
    /// as it is, and a stop or a create of the same identity releases it (SOMA#220).
    Stopped,
}
