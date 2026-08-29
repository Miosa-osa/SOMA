use std::fmt;

use crate::{GenerationId, InstanceId, MachineSpec, OperationId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Milestone {
    RequestAccepted,
    ArtifactsVerified,
    MachineRestored,
    GuestAuthenticated,
    GenerationAcknowledged,
    IdentityRepaired,
    NetworkRepaired,
    FirstCommandSucceeded,
    Ready,
    RollbackStarted,
    CleanupCompleted,
    StopRequested,
    Stopped,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Milestones(Vec<Milestone>);

impl Milestones {
    pub(crate) fn push(&mut self, milestone: Milestone) {
        self.0.push(milestone);
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Milestone] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ready {
    operation_id: OperationId,
    instance_id: InstanceId,
    generation_id: GenerationId,
    machine: MachineSpec,
    milestones: Milestones,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    Code(i32),
    Signal(u8),
    TimedOut,
    OutputLimit,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Executed {
    operation_id: OperationId,
    instance_id: InstanceId,
    status: ExitStatus,
    stdout: Box<[u8]>,
    stderr: Box<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stopped {
    operation_id: OperationId,
    instance_id: InstanceId,
    guest_acknowledged: bool,
    forced: bool,
    cleanup: crate::CleanupEvidence,
    milestones: Milestones,
}

impl Stopped {
    pub(crate) const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        guest_acknowledged: bool,
        forced: bool,
        cleanup: crate::CleanupEvidence,
        milestones: Milestones,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            guest_acknowledged,
            forced,
            cleanup,
            milestones,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn guest_acknowledged(&self) -> bool {
        self.guest_acknowledged
    }

    #[must_use]
    pub const fn forced(&self) -> bool {
        self.forced
    }

    #[must_use]
    pub const fn cleanup(&self) -> crate::CleanupEvidence {
        self.cleanup
    }

    #[must_use]
    pub fn milestones(&self) -> &[Milestone] {
        self.milestones.as_slice()
    }
}

impl Executed {
    pub(crate) fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            status,
            stdout: stdout.into_boxed_slice(),
            stderr: stderr.into_boxed_slice(),
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn status(&self) -> ExitStatus {
        self.status
    }

    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

impl fmt::Debug for Executed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Executed")
            .field("operation_id", &self.operation_id)
            .field("instance_id", &self.instance_id)
            .field("status", &self.status)
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .finish()
    }
}

impl Ready {
    pub(crate) const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        generation_id: GenerationId,
        machine: MachineSpec,
        milestones: Milestones,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            generation_id,
            machine,
            milestones,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn generation_id(&self) -> GenerationId {
        self.generation_id
    }

    #[must_use]
    pub const fn machine(&self) -> MachineSpec {
        self.machine
    }

    #[must_use]
    pub fn milestones(&self) -> &[Milestone] {
        self.milestones.as_slice()
    }
}
