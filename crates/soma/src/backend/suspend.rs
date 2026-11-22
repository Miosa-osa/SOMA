//! The backend half of a suspension (ADR 0034): the guest retires its session and parks, and the
//! machine is read into the staging directory a Suspension store seals from.

use std::path::Path;

use crate::{InstanceId, OperationId};

/// One exact Instance to suspend, and where its plaintext members are staged.
#[derive(Clone, Copy)]
pub struct SuspendRequest<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    staging: &'a Path,
}

impl<'a> SuspendRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        staging: &'a Path,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            staging,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    /// The directory the machine's memory, overlay and state are read into.
    #[must_use]
    pub const fn staging(&self) -> &Path {
        self.staging
    }
}

/// What a suspension came to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuspendObservation {
    /// The machine was read into the staging directory and no longer runs.
    Suspended,
    /// Refused with nothing sent: the Instance cannot be suspended (its guest agent is too old,
    /// or its machine was not restored from a snapshot). It runs exactly as before.
    Refused,
}
