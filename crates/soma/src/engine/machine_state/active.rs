use std::collections::BTreeSet;

use crate::{
    BackendKind, InstanceId, MachineName, MachineShape, OperationId, OperationKind,
    StateStoreFailure, TerminalStatus, WorkloadEvidence, WorkloadIdentity,
};

use super::{ActiveMachine, ExecutionTombstone, MAX_EXECUTION_TOMBSTONES, corrupt};

impl ActiveMachine {
    pub(super) fn validate(&self, instance_id: &InstanceId) -> Result<(), StateStoreFailure> {
        if self.launch_receipt.operation() != OperationKind::Launch
            || self.launch_receipt.instance_id() != instance_id
            || self.launch_receipt.terminal_status() != &TerminalStatus::Ready
            || self.completed_executions.len() > MAX_EXECUTION_TOMBSTONES
        {
            return Err(corrupt());
        }
        let mut operation_ids = BTreeSet::new();
        for completed in &self.completed_executions {
            if !operation_ids.insert(completed.operation_id.clone())
                || !matches!(
                    completed.terminal_status,
                    TerminalStatus::Exited { .. } | TerminalStatus::Signaled { .. }
                )
            {
                return Err(corrupt());
            }
        }
        Ok(())
    }

    /// Keeps a completed execution's tombstone, forgetting the oldest when the ledger is full.
    ///
    /// A full ledger used to refuse every later command ("full replay ledger requires
    /// replacement"), so a sandbox could run 1,024 commands in its life and then none (seen
    /// 2026-11-23 on desktops polled for screenshots). A tombstone answers a retried operation
    /// instead of running it twice; retries come within seconds, so the oldest tombstones are
    /// the ones nothing can still be retrying, and they make room.
    pub(in crate::engine) fn record_completed(&mut self, tombstone: ExecutionTombstone) {
        while self.completed_executions.len() >= MAX_EXECUTION_TOMBSTONES {
            self.completed_executions.remove(0);
        }
        self.completed_executions.push(tombstone);
    }

    pub(in crate::engine) fn workload(&self) -> Option<&WorkloadIdentity> {
        match self.launch_receipt.workload() {
            WorkloadEvidence::Resolved { identity } => Some(identity),
            WorkloadEvidence::Unresolved { .. } => None,
        }
    }

    pub(in crate::engine) fn machine_name(&self) -> Option<&MachineName> {
        self.launch_receipt.machine_name()
    }

    pub(in crate::engine) fn shape(&self) -> &MachineShape {
        self.launch_receipt.requested_shape()
    }

    pub(in crate::engine) fn backend(&self) -> BackendKind {
        self.launch_receipt.backend()
    }

    pub(in crate::engine) fn completed(
        &self,
        operation_id: &OperationId,
    ) -> Option<&ExecutionTombstone> {
        self.completed_executions
            .iter()
            .find(|completed| completed.operation_id == *operation_id)
    }
}
