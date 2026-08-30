//! Idempotent release in the specification order.
//!
//! Ingress, forwarding, conntrack zone, routes and addresses, veth, TAP, namespace, host
//! ruleset, reservations, and finally the ledger record; every step reports whether it removed
//! something, found it already absent, or failed, and a final live inspection decides
//! `complete`.
//!
//! Teardown is total rather than fail-fast: a step that fails is recorded and the remaining
//! steps still run, so one wedged tool can never leave the namespace, the veth, and the host
//! ruleset behind. An incomplete release writes no ledger release record, so reconciliation
//! still owns the remains and a replayed release retries exactly the steps that failed.

use std::{os::fd::OwnedFd, path::Path};

use crate::{
    Assigned, AssignmentRecord, Broker, BundleId, BundleNames, CleanupGeneration, ConntrackZone,
    Error, PortReservation, SterileBundle, link,
    namespace::{NetNamespace, Unpinned},
    netlink, nft, sysctl,
};

/// The disposition of one release step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepResult {
    /// The resource was removed now.
    Removed,
    /// The resource was already absent.
    AlreadyAbsent,
    /// The resource disappears with its parent object.
    RemovedWithParent,
    /// The step was skipped because a prerequisite was absent.
    Skipped,
    /// The step failed; the resource may still exist.
    Failed,
}

/// What release did, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseEvidence {
    /// Port reservations dropped.
    pub ingress: usize,
    /// Forwarding disabled inside the namespace.
    pub forwarding: StepResult,
    /// Conntrack zone flushed on the host.
    pub conntrack: StepResult,
    /// Routes and addresses.
    pub routes: StepResult,
    /// The veth pair.
    pub veth: StepResult,
    /// The TAP descriptor.
    pub tap: StepResult,
    /// The namespace pin.
    pub namespace: StepResult,
    /// The host ruleset.
    pub host_ruleset: StepResult,
    /// Whether the ledger recorded the release.
    pub ledger: bool,
    /// Whether the final live inspection found no owned resource and every step succeeded.
    pub complete: bool,
    /// The first step failure, if any; the later steps ran regardless.
    pub failure: Option<Error>,
}

/// Releases one assigned bundle.
#[must_use]
pub fn release(broker: &Broker, assigned: Assigned) -> ReleaseEvidence {
    let Assigned {
        bundle,
        reservations,
        ..
    } = assigned;
    release_sterile(broker, bundle, reservations)
}

/// Releases one sterile bundle that was never assigned.
#[must_use]
pub fn release_sterile(
    broker: &Broker,
    bundle: SterileBundle,
    reservations: Vec<PortReservation>,
) -> ReleaseEvidence {
    let SterileBundle {
        id,
        generation,
        names,
        namespace,
        tap,
        zone,
        ..
    } = bundle;
    let path = namespace.path().to_path_buf();
    drop(namespace);
    let mut evidence = teardown(&names, zone, &path, Some(tap), reservations);
    if evidence.complete {
        evidence.ledger = record(broker, id, generation);
    }
    evidence
}

/// Releases whatever the ledger record still owns after a crash; the TAP descriptor is gone
/// with the process that held it.
#[must_use]
pub fn release_record(broker: &Broker, record_entry: &AssignmentRecord) -> ReleaseEvidence {
    let names = BundleNames::new(&record_entry.bundle.short_hex());
    let path = broker.namespace_dir().join(record_entry.bundle.short_hex());
    let mut evidence = teardown(&names, record_entry.zone, &path, None, Vec::new());
    if evidence.complete {
        evidence.ledger = record(broker, record_entry.bundle, record_entry.generation);
    }
    evidence
}

fn record(broker: &Broker, id: BundleId, generation: CleanupGeneration) -> bool {
    broker.ledger().record_release(id, generation).is_ok()
}

pub(crate) fn teardown(
    names: &BundleNames,
    zone: ConntrackZone,
    pin: &Path,
    tap: Option<OwnedFd>,
    reservations: Vec<PortReservation>,
) -> ReleaseEvidence {
    let ingress = reservations.len();
    drop(reservations);
    let mut failure = None;
    let forwarding = step(&mut failure, || {
        if !pin.exists() {
            return Ok(StepResult::Skipped);
        }
        NetNamespace::open(pin)?.within(|| sysctl::set_forwarding(false))?;
        Ok(StepResult::Removed)
    });
    let conntrack = step(&mut failure, || {
        nft::flush_zone(zone).map(|()| StepResult::Removed)
    });
    let veth = step(&mut failure, || {
        Ok(removed(netlink::delete_link(&names.host_veth)?))
    });
    let tap = match tap {
        Some(fd) => {
            drop(fd);
            StepResult::Removed
        }
        None => StepResult::Skipped,
    };
    let namespace = step(&mut failure, || {
        Ok(match NetNamespace::unpin(pin)? {
            Unpinned::Removed => StepResult::Removed,
            Unpinned::AlreadyAbsent => StepResult::AlreadyAbsent,
        })
    });
    let host_ruleset = step(&mut failure, || {
        Ok(removed(nft::delete_table(&names.host_table)?))
    });
    let inspected = step(&mut failure, || {
        let clean = !pin.exists()
            && !link::list_links()?.contains(&names.host_veth)
            && !nft::table_exists(&names.host_table)?;
        Ok(if clean {
            StepResult::Removed
        } else {
            StepResult::Failed
        })
    });
    ReleaseEvidence {
        ingress,
        forwarding,
        conntrack,
        routes: StepResult::RemovedWithParent,
        veth,
        tap,
        namespace,
        host_ruleset,
        ledger: false,
        complete: failure.is_none() && inspected == StepResult::Removed,
        failure,
    }
}

/// Runs one teardown step, recording the first failure and never aborting the sequence.
fn step(
    failure: &mut Option<Error>,
    run: impl FnOnce() -> Result<StepResult, Error>,
) -> StepResult {
    match run() {
        Ok(result) => result,
        Err(error) => {
            failure.get_or_insert(error);
            StepResult::Failed
        }
    }
}

const fn removed(deleted: bool) -> StepResult {
    if deleted {
        StepResult::Removed
    } else {
        StepResult::AlreadyAbsent
    }
}

#[cfg(test)]
mod tests;
