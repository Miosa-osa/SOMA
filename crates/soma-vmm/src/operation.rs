use std::collections::HashMap;

use crate::{Execute, Executed, Failure, Launch, OperationId, Ready, Stop, Stopped};

/// Maximum terminal operation receipts retained by one Machine process.
pub const MAX_OPERATION_RECEIPTS: usize = 1_024;
/// Maximum logical request and output bytes retained for exact Execute replay.
pub const MAX_OPERATION_RECEIPT_BYTES: usize = 64 * 1024 * 1024;

const MAX_EXECUTE_RECEIPTS: usize = MAX_OPERATION_RECEIPTS - 2;

#[derive(Default)]
pub(super) struct OperationLedger {
    launch: Option<LaunchRecord>,
    executes: HashMap<OperationId, ExecuteRecord>,
    stop: Option<StopRecord>,
    retained_execute_bytes: usize,
}

impl OperationLedger {
    pub(super) fn ensure_launch_capacity(&self) -> Result<(), OperationCapacity> {
        if self.launch.is_some() {
            Err(OperationCapacity)
        } else {
            Ok(())
        }
    }

    pub(super) fn ensure_execute_capacity(
        &self,
        request: &Execute,
    ) -> Result<(), OperationCapacity> {
        if self.executes.len() >= MAX_EXECUTE_RECEIPTS {
            return Err(OperationCapacity);
        }
        let retained_bytes = self
            .retained_execute_bytes
            .checked_add(maximum_execute_receipt_bytes(request))
            .ok_or(OperationCapacity)?;
        if retained_bytes > MAX_OPERATION_RECEIPT_BYTES {
            return Err(OperationCapacity);
        }
        Ok(())
    }

    pub(super) fn replay_launch(
        &self,
        request: &Launch,
    ) -> Result<Option<Result<Ready, Failure>>, OperationConflict> {
        if self.has_execute_id(request.operation_id()) || self.has_stop_id(request.operation_id()) {
            return Err(OperationConflict);
        }
        let Some(record) = &self.launch else {
            return Ok(None);
        };
        if record.request.operation_id() != request.operation_id() {
            return Ok(None);
        }
        if record.request == *request {
            Ok(Some(record.outcome.clone()))
        } else {
            Err(OperationConflict)
        }
    }

    pub(super) fn record_launch(&mut self, request: Launch, outcome: Result<Ready, Failure>) {
        debug_assert!(self.launch.is_none(), "Launch receipt already exists");
        self.launch = Some(LaunchRecord { request, outcome });
    }

    pub(super) fn replay_execute(
        &self,
        request: &Execute,
    ) -> Result<Option<Result<Executed, Failure>>, OperationConflict> {
        if self.has_launch_id(request.operation_id()) || self.has_stop_id(request.operation_id()) {
            return Err(OperationConflict);
        }
        let Some(record) = self.executes.get(&request.operation_id()) else {
            return Ok(None);
        };
        if record.request == *request {
            Ok(Some(record.outcome.clone()))
        } else {
            Err(OperationConflict)
        }
    }

    pub(super) fn record_execute(&mut self, request: Execute, outcome: Result<Executed, Failure>) {
        let retained_bytes = execute_receipt_bytes(&request, &outcome);
        self.retained_execute_bytes = self.retained_execute_bytes.saturating_add(retained_bytes);
        debug_assert!(self.retained_execute_bytes <= MAX_OPERATION_RECEIPT_BYTES);
        let replaced = self
            .executes
            .insert(request.operation_id(), ExecuteRecord { request, outcome });
        debug_assert!(
            replaced.is_none(),
            "new Execute replaced an existing receipt"
        );
    }

    pub(super) fn replay_stop(&self, request: &Stop) -> Result<StopReplay, OperationConflict> {
        if self.has_launch_id(request.operation_id()) || self.has_execute_id(request.operation_id())
        {
            return Err(OperationConflict);
        }
        let Some(record) = &self.stop else {
            return Ok(StopReplay::New);
        };
        match record {
            StopRecord::Reaping(recorded) if recorded == request => Ok(StopReplay::Continue),
            StopRecord::Complete {
                request: recorded,
                outcome,
            } if recorded == request => Ok(StopReplay::Complete(outcome.clone())),
            StopRecord::Reaping(_) | StopRecord::Complete { .. } => Err(OperationConflict),
        }
    }

    pub(super) fn admit_stop(&mut self, request: Stop) {
        debug_assert!(self.stop.is_none(), "Stop operation already admitted");
        self.stop = Some(StopRecord::Reaping(request));
    }

    pub(super) fn complete_stop(&mut self, outcome: Result<Stopped, Failure>) {
        let Some(StopRecord::Reaping(request)) = self.stop.take() else {
            debug_assert!(false, "Stop completed without an admitted operation");
            return;
        };
        self.stop = Some(StopRecord::Complete { request, outcome });
    }

    fn has_launch_id(&self, operation_id: OperationId) -> bool {
        self.launch
            .as_ref()
            .is_some_and(|record| record.request.operation_id() == operation_id)
    }

    fn has_execute_id(&self, operation_id: OperationId) -> bool {
        self.executes.contains_key(&operation_id)
    }

    fn has_stop_id(&self, operation_id: OperationId) -> bool {
        self.stop
            .as_ref()
            .is_some_and(|record| record.request().operation_id() == operation_id)
    }
}

fn maximum_execute_receipt_bytes(request: &Execute) -> usize {
    execute_request_bytes(request).saturating_add(output_limit(request))
}

fn execute_receipt_bytes(request: &Execute, outcome: &Result<Executed, Failure>) -> usize {
    let output_bytes = outcome.as_ref().map_or(0, |executed| {
        executed
            .stdout()
            .len()
            .saturating_add(executed.stderr().len())
    });
    execute_request_bytes(request).saturating_add(output_bytes)
}

fn execute_request_bytes(request: &Execute) -> usize {
    request
        .arguments()
        .iter()
        .fold(request.program().as_bytes().len(), |total, argument| {
            total.saturating_add(argument.as_bytes().len())
        })
}

fn output_limit(request: &Execute) -> usize {
    usize::try_from(request.limits().output().get()).unwrap_or(usize::MAX)
}

struct LaunchRecord {
    request: Launch,
    outcome: Result<Ready, Failure>,
}

struct ExecuteRecord {
    request: Execute,
    outcome: Result<Executed, Failure>,
}

enum StopRecord {
    Reaping(Stop),
    Complete {
        request: Stop,
        outcome: Result<Stopped, Failure>,
    },
}

impl StopRecord {
    const fn request(&self) -> &Stop {
        match self {
            Self::Reaping(request) | Self::Complete { request, .. } => request,
        }
    }
}

pub(super) enum StopReplay {
    New,
    Continue,
    Complete(Result<Stopped, Failure>),
}

pub(super) struct OperationConflict;

pub(super) struct OperationCapacity;
