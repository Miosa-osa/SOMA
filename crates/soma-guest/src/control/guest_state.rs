use crate::OperationId;

use super::{error::ControlStage, exchange::OutputAccounting};

pub(super) enum GuestState {
    AwaitPrepare(OperationId),
    PrepareReporting(OperationId),
    RepairedIdle,
    ExecuteStreaming(ActiveExchange),
    FilePending(OperationId),
    PtyPending(OperationId),
    ShutdownPending(OperationId),
}

pub(super) struct ActiveExchange {
    pub(super) operation: OperationId,
    pub(super) accounting: OutputAccounting,
}

pub(super) fn active_stage(state: &GuestState) -> ControlStage {
    match state {
        GuestState::AwaitPrepare(_) | GuestState::PrepareReporting(_) => ControlStage::Repair,
        GuestState::RepairedIdle | GuestState::ExecuteStreaming(_) => ControlStage::Execute,
        GuestState::FilePending(_) => ControlStage::File,
        GuestState::PtyPending(_) => ControlStage::Pty,
        GuestState::ShutdownPending(_) => ControlStage::Shutdown,
    }
}

pub(super) fn receive_stage(state: &GuestState) -> ControlStage {
    match state {
        GuestState::AwaitPrepare(_) => ControlStage::Repair,
        GuestState::RepairedIdle => ControlStage::Execute,
        _ => active_stage(state),
    }
}
