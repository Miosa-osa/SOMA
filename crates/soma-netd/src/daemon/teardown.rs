//! A release request: one verified teardown, answered only to the peer that claimed it.
//!
//! A release first waits a bounded moment for the request path to go quiet and then holds one
//! teardown permit, so a burst of releases neither stalls claims nor overruns the tool deadline.
//! A bundle only the ledger still names, as every live Machine is after a broker restart, is
//! released from its durable record, and only to the peer that record names.

use std::time::Duration;

use crate::{Error, PeerIdentity, Reply, error_code, release, release_record};

use super::{Slot, State, operation_of};

/// The longest a release waits for the request path to go quiet before it runs anyway.
const RELEASE_PATIENCE: Duration = Duration::from_millis(500);

pub(super) fn release_request(state: &State, peer: PeerIdentity, slot: Slot) -> Reply {
    let (bundle, generation) = slot;
    state.traffic.wait_quiet(RELEASE_PATIENCE);
    let _turn = state.teardowns.acquire();
    let result = match state.disown((bundle, generation)) {
        Some(owned) if owned.peer != peer.uid() => {
            state.own(owned);
            Err(Error::Unauthorized("assignment owner"))
        }
        Some(owned) => {
            let operation = operation_of(&owned.assigned);
            let record = owned.assigned.record().clone();
            let evidence = release(&state.broker, owned.assigned);
            log_release(&record, &evidence);
            if evidence.complete {
                state.forget(operation);
                state.recycle(&record);
            }
            Ok(evidence)
        }
        None => match state.broker.ledger().lookup(bundle, generation) {
            Ok(entry) if entry.record.owner != peer.uid() => {
                Err(Error::Unauthorized("assignment owner"))
            }
            Ok(entry) => {
                let evidence = release_record(&state.broker, &entry.record);
                if evidence.complete {
                    state.forget((entry.record.instance, entry.record.operation));
                    if !entry.released {
                        state.recycle(&entry.record);
                    }
                }
                Ok(evidence)
            }
            Err(error) => Err(error),
        },
    };
    match result {
        Ok(evidence) => Reply::Released {
            complete: evidence.complete,
        },
        Err(error) => Reply::Failed(error_code(&error)),
    }
}

/// Says why a release was not verified complete; reconciliation owns what is left.
fn log_release(record: &crate::AssignmentRecord, evidence: &crate::ReleaseEvidence) {
    if let Some(usage) = evidence.usage {
        eprintln!(
            "soma-netd: usage bundle={} bytes_out={} bytes_in={} packets_out={} packets_in={}",
            record.bundle.short_hex(),
            usage.bytes_out,
            usage.bytes_in,
            usage.packets_out,
            usage.packets_in
        );
    }
    if !evidence.complete {
        eprintln!("soma-netd: release incomplete: {evidence:?}");
    }
}
