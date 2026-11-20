//! Putting a bundle claimed with no egress back to that base after the broker restarted.
//!
//! The kernel keeps a bundle's sandbox ruleset across a broker restart, but the broker's record of
//! which window is open lives in memory and is gone. A window that was open at the restart would
//! stay open with nothing left to close it. So at start, before the control socket is bound,
//! every unreleased assignment whose recorded base intent denies egress is rendered back to that
//! base and its tracked flows are flushed. A bundle whose base allows egress is not touched: its
//! ruleset is what it was claimed with, and cutting it would take the internet from sandboxes that
//! were never meant to lose it.
//!
//! If the base cannot be rendered back, forwarding is switched off in that namespace instead. The
//! table is never deleted: a deleted table filters nothing, and forwarding is on.

use crate::{
    AssignmentRecord, Broker, BundleNames, Error, SandboxRuleset, namespace::NetNamespace, nft,
    sysctl,
};

/// What a start did with the bundles it found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RestoreReport {
    /// Bundles whose base ruleset was rendered back and whose tracked flows were flushed.
    pub restored: Vec<String>,
    /// Bundles whose base could not be rendered back, so forwarding was switched off instead.
    pub forwarding_off: Vec<String>,
    /// Bundles that could be neither restored nor cut off. These need an operator.
    pub failed: Vec<String>,
}

/// Returns every unreleased bundle claimed with no egress to that base.
///
/// # Errors
///
/// Returns a ledger read failure; nothing is changed then.
pub fn restore_base_rulesets(broker: &Broker) -> Result<RestoreReport, Error> {
    let mut report = RestoreReport::default();
    for entry in broker.ledger().entries()? {
        if entry.released || entry.record.intent.egress().forwards() {
            continue;
        }
        let short = entry.record.bundle.short_hex();
        if !broker.namespace_dir().join(&short).exists() {
            // The namespace is gone, so there is no ruleset to restore; reconciliation reports
            // the entry as drifted.
            continue;
        }
        if restore_one(broker, &entry.record).is_ok() {
            report.restored.push(short);
        } else if cut_forwarding(broker, &entry.record).is_ok() {
            report.forwarding_off.push(short);
        } else {
            report.failed.push(short);
        }
    }
    Ok(report)
}

fn restore_one(broker: &Broker, record: &AssignmentRecord) -> Result<(), Error> {
    let short = record.bundle.short_hex();
    let names = BundleNames::new(&short);
    let lease = broker
        .profile()
        .leases()
        .lease(record.lease_index)
        .ok_or(Error::InvalidState("lease index"))?;
    let ruleset = SandboxRuleset {
        names: &names,
        lease,
        guest_mac: record.guest_mac,
        intent: &record.intent,
        protected: broker.profile().protected(),
        // Published mappings were reserved by the previous process and are not recoverable; a
        // base that denied egress renders without them.
        published: &[],
        connect_ports: broker.connect_ports().ranges(),
    }
    .render();
    let namespace = NetNamespace::open(&broker.namespace_dir().join(&short))?;
    namespace.within(move || nft::apply(&ruleset))?;
    namespace.within(nft::flush_all)?;
    nft::flush_zone(record.zone)
}

fn cut_forwarding(broker: &Broker, record: &AssignmentRecord) -> Result<(), Error> {
    NetNamespace::open(&broker.namespace_dir().join(record.bundle.short_hex()))?
        .within(|| sysctl::set_forwarding(false))
}
