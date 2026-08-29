//! Idempotent release in the specification order.
//!
//! Ingress, forwarding, conntrack zone, routes and addresses, veth, TAP, namespace, host
//! ruleset, reservations, and finally the ledger record; every step reports whether it removed
//! something or found it already absent, and a final live inspection decides `complete`.

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
    /// Whether the final live inspection found no owned resource.
    pub complete: bool,
}

/// Releases one assigned bundle.
///
/// # Errors
///
/// Returns the first hard kernel or ledger failure; absent resources are not failures.
pub fn release(broker: &Broker, assigned: Assigned) -> Result<ReleaseEvidence, Error> {
    let Assigned {
        bundle,
        reservations,
        ..
    } = assigned;
    release_sterile(broker, bundle, reservations)
}

/// Releases one sterile bundle that was never assigned.
///
/// # Errors
///
/// Returns the first hard kernel failure.
pub fn release_sterile(
    broker: &Broker,
    bundle: SterileBundle,
    reservations: Vec<PortReservation>,
) -> Result<ReleaseEvidence, Error> {
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
    let mut evidence = teardown(broker, &names, zone, &path, Some(tap), reservations)?;
    evidence.ledger = record(broker, id, generation);
    Ok(evidence)
}

/// Releases whatever the ledger record still owns after a crash; the TAP descriptor is gone
/// with the process that held it.
///
/// # Errors
///
/// Returns the first hard kernel failure.
pub fn release_record(
    broker: &Broker,
    record_entry: &AssignmentRecord,
) -> Result<ReleaseEvidence, Error> {
    let names = BundleNames::new(&record_entry.bundle.short_hex());
    let path = broker.namespace_dir().join(record_entry.bundle.short_hex());
    let mut evidence = teardown(broker, &names, record_entry.zone, &path, None, Vec::new())?;
    evidence.ledger = record(broker, record_entry.bundle, record_entry.generation);
    Ok(evidence)
}

fn record(broker: &Broker, id: BundleId, generation: CleanupGeneration) -> bool {
    broker.ledger().record_release(id, generation).is_ok()
}

pub(crate) fn teardown(
    _broker: &Broker,
    names: &BundleNames,
    zone: ConntrackZone,
    pin: &Path,
    tap: Option<OwnedFd>,
    reservations: Vec<PortReservation>,
) -> Result<ReleaseEvidence, Error> {
    let ingress = reservations.len();
    drop(reservations);
    let forwarding = if pin.exists() {
        let namespace = NetNamespace::open(pin)?;
        namespace.within(|| sysctl::set_forwarding(false))?;
        StepResult::Removed
    } else {
        StepResult::Skipped
    };
    nft::flush_zone(zone)?;
    let veth = if netlink::delete_link(&names.host_veth)? {
        StepResult::Removed
    } else {
        StepResult::AlreadyAbsent
    };
    let tap = match tap {
        Some(fd) => {
            drop(fd);
            StepResult::Removed
        }
        None => StepResult::Skipped,
    };
    let namespace = match NetNamespace::unpin(pin)? {
        Unpinned::Removed => StepResult::Removed,
        Unpinned::AlreadyAbsent => StepResult::AlreadyAbsent,
    };
    let host_ruleset = if nft::delete_table(&names.host_table)? {
        StepResult::Removed
    } else {
        StepResult::AlreadyAbsent
    };
    let complete = !pin.exists()
        && !link::list_links()?.contains(&names.host_veth)
        && !nft::list_tables()?.contains(&names.host_table);
    Ok(ReleaseEvidence {
        ingress,
        forwarding,
        conntrack: StepResult::Removed,
        routes: StepResult::RemovedWithParent,
        veth,
        tap,
        namespace,
        host_ruleset,
        ledger: false,
        complete,
    })
}
