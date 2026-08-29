//! The projection of ordered records into one entry per worker.
//!
//! Every record that names resources refreshes the entry's references, so reconciliation
//! after a crash releases what the worker actually held rather than what it held when it was
//! sterile.

use std::collections::BTreeMap;

use super::{LedgerError, Record, RecordKind};
use crate::{Phase, ReconcileDisposition, TransferStep, WorkerId, WorkerLedgerEntry};

pub(super) fn fold(
    entries: &mut BTreeMap<WorkerId, WorkerLedgerEntry>,
    record: &Record,
) -> Result<(), LedgerError> {
    let invariant = LedgerError::Invariant {
        worker: record.worker,
        kind: record.kind,
    };
    if record.kind == RecordKind::Constructing {
        if entries.contains_key(&record.worker) {
            return Err(invariant);
        }
        entries.insert(record.worker, fresh(record));
        return Ok(());
    }
    let entry = entries.get_mut(&record.worker).ok_or(invariant)?;
    if entry.phase == Phase::Dead {
        return Err(invariant);
    }
    entry.records += 1;
    entry.lease_generation = record.lease_generation;
    match record.kind {
        RecordKind::Constructing => return Err(invariant),
        RecordKind::Sterile => {
            if entry.was_assigned || entry.phase != Phase::Constructing {
                return Err(invariant);
            }
            entry.phase = Phase::Sterile;
            entry.identity = record.identity;
            entry.resources = record.resources;
        }
        RecordKind::ConstructFailed | RecordKind::Dead => entry.phase = Phase::Dead,
        RecordKind::Claiming => {
            entry.phase = Phase::Claiming;
            entry.operation = record.operation;
            entry.fingerprint = record.fingerprint;
        }
        RecordKind::Assigning => entry.resources = record.resources,
        RecordKind::TransferStep => {
            entry.last_step = TransferStep::from_code(record.detail);
            entry.resources = record.resources;
        }
        RecordKind::TransferFault => {}
        RecordKind::Assigned => {
            entry.phase = Phase::Assigned;
            entry.instance = record.instance;
            entry.resources = record.resources;
            entry.was_assigned = true;
        }
        RecordKind::Running => entry.phase = Phase::Running,
        RecordKind::Destroying => entry.phase = Phase::Destroying,
        RecordKind::Suspect => entry.suspect = true,
        RecordKind::Reconciled => {
            if ReconcileDisposition::from_code(record.detail)
                != Some(ReconcileDisposition::Retained)
            {
                entry.phase = Phase::Dead;
            }
        }
    }
    Ok(())
}

fn fresh(record: &Record) -> WorkerLedgerEntry {
    WorkerLedgerEntry {
        worker: record.worker,
        key: record.key,
        phase: Phase::Constructing,
        lease_generation: record.lease_generation,
        operation: None,
        instance: None,
        fingerprint: None,
        resources: record.resources,
        identity: record.identity,
        last_step: None,
        was_assigned: false,
        suspect: false,
        records: 1,
    }
}
