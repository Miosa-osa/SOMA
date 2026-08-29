//! The projection of ordered records into one entry per worker.
//!
//! Every record that names resources refreshes the entry's references, so reconciliation
//! after a crash releases what the worker actually held rather than what it held when it was
//! sterile.
//! Every phase-changing record must follow the legal transition table and keep the lease
//! generation, which only a claim bumps, so the durable ledger proves on its own that no
//! worker was ever reclaimed.

use std::collections::BTreeMap;

use super::{LedgerError, Record, RecordKind};
use crate::{
    LeaseGeneration, Phase, ReconcileDisposition, TransferStep, WorkerId, WorkerLedgerEntry,
};

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
    if !generation_follows(entry.lease_generation, record) {
        return Err(invariant);
    }
    if let Some(next) = moves_to(record.kind)
        && !entry.phase.may_transition_to(next)
    {
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

/// The phase a record moves an entry to, when it moves one.
///
/// A reconciliation that did not retain the worker closes it from whatever live phase the
/// restart found, which is the one transition the phase table does not describe.
const fn moves_to(kind: RecordKind) -> Option<Phase> {
    match kind {
        RecordKind::Sterile => Some(Phase::Sterile),
        RecordKind::ConstructFailed | RecordKind::Dead => Some(Phase::Dead),
        RecordKind::Claiming => Some(Phase::Claiming),
        RecordKind::Assigned => Some(Phase::Assigned),
        RecordKind::Running => Some(Phase::Running),
        RecordKind::Destroying => Some(Phase::Destroying),
        RecordKind::Constructing
        | RecordKind::Assigning
        | RecordKind::TransferStep
        | RecordKind::TransferFault
        | RecordKind::Suspect
        | RecordKind::Reconciled => None,
    }
}

/// The claim is the one record that bumps the lease generation; nothing else may change it.
fn generation_follows(current: LeaseGeneration, record: &Record) -> bool {
    let expected = if record.kind == RecordKind::Claiming {
        current.next().ok()
    } else {
        Some(current)
    };
    expected == Some(record.lease_generation)
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
