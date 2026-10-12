//! Reconciliation of the durable ledger against kernel reality.
//!
//! Every ledger entry is compared with the namespace pin, the host veth, and the host table it
//! should own; kernel objects carrying SOMA names without a ledger owner are reported but never
//! removed, because ownership must be proven before cleanup.

use std::{collections::BTreeSet, fs, os::unix::fs::MetadataExt, path::Path};

use crate::{
    Broker, BundleId, BundleNames, CleanupGeneration, Drift, Error, Step, link,
    namespace::NetNamespace, netlink, nft,
};

/// The disposition of one ledger entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    /// Assigned and every owned kernel object is present.
    Consistent,
    /// Released and no owned kernel object remains.
    Released,
    /// Assigned but an owned kernel object is missing; the lease must be released.
    Drifted(Drift),
    /// Released but an owned kernel object remains; cleanup must be repeated.
    Orphaned,
}

/// The reconciliation report.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconcileReport {
    /// Every ledger entry with its disposition.
    pub entries: Vec<(BundleId, CleanupGeneration, Disposition)>,
    /// Namespace pins without a ledger owner.
    pub unowned_namespaces: Vec<String>,
    /// Host veth links with the broker prefix and no ledger owner.
    pub unowned_links: Vec<String>,
    /// Host tables with the broker prefix and no ledger owner.
    pub unowned_tables: Vec<String>,
}

impl ReconcileReport {
    /// Counts entries by disposition class: consistent, drifted, orphaned.
    #[must_use]
    pub fn counts(&self) -> (u32, u32, u32) {
        let mut counts = (0, 0, 0);
        for (_, _, disposition) in &self.entries {
            match disposition {
                Disposition::Consistent | Disposition::Released => counts.0 += 1,
                Disposition::Drifted(_) => counts.1 += 1,
                Disposition::Orphaned => counts.2 += 1,
            }
        }
        counts
    }

    /// Counts unowned kernel objects.
    #[must_use]
    pub fn unowned(&self) -> u32 {
        u32::try_from(
            self.unowned_namespaces.len() + self.unowned_links.len() + self.unowned_tables.len(),
        )
        .unwrap_or(u32::MAX)
    }
}

/// Compares the ledger with the kernel.
///
/// # Errors
///
/// Returns a ledger read failure or a kernel listing failure.
pub fn reconcile(broker: &Broker) -> Result<ReconcileReport, Error> {
    let pins = NetNamespace::list(broker.namespace_dir())?;
    let links = link::list_links()?;
    let tables = nft::list_tables()?;
    let mut report = ReconcileReport::default();
    let mut owned_short = Vec::new();
    for entry in broker.ledger().entries()? {
        let record = entry.record;
        let short = record.bundle.short_hex();
        let names = BundleNames::new(&short);
        let pin = pins.contains(&short);
        let veth = links.contains(&names.host_veth);
        let table = tables.contains(&names.host_table);
        // A leaked publication table would otherwise be invisible here, because it is not one
        // of the three objects an assigned entry must own; a released entry that still has one
        // is orphaned exactly as it would be with a leftover host table.
        let published = tables.contains(&names.publication_table);
        let disposition = if entry.released {
            if pin || veth || table || published {
                Disposition::Orphaned
            } else {
                Disposition::Released
            }
        } else if !pin {
            Disposition::Drifted(Drift::NamespaceMissing)
        } else if !veth {
            Disposition::Drifted(Drift::HostVethMissing)
        } else if !table {
            Disposition::Drifted(Drift::HostRulesetMissing)
        } else {
            Disposition::Consistent
        };
        report
            .entries
            .push((record.bundle, record.generation, disposition));
        owned_short.push(short);
    }
    report.unowned_namespaces = pins
        .into_iter()
        .filter(|name| !owned_short.contains(name))
        .collect();
    report.unowned_links = links
        .into_iter()
        .filter(|name| name.strip_prefix("sv").is_some_and(|rest| rest.len() == 8))
        .filter(|name| {
            !owned_short
                .iter()
                .any(|short| format!("sv{short}") == *name)
        })
        .collect();
    report.unowned_tables = tables
        .into_iter()
        .filter(|name| {
            name.starts_with("soma_") || name.starts_with("somah_") || name.starts_with("somap_")
        })
        .filter(|name| {
            !owned_short.iter().any(|short| {
                *name == format!("soma_{short}")
                    || *name == format!("somah_{short}")
                    || *name == format!("somap_{short}")
            })
        })
        .collect();
    Ok(report)
}

/// What [`reap_unassigned`] did with the kernel objects of bundles no ledger record owns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReapReport {
    /// Bundle names whose objects were all removed and verified absent.
    pub removed: Vec<String>,
    /// Bundle names left alone because a process still has their network namespace open.
    pub in_use: Vec<String>,
    /// Bundle names where a removal step failed or an object remained afterwards.
    pub failed: Vec<String>,
}

/// Removes the sterile bundles a previous broker process prepared and never assigned.
///
/// A broker prepares a pool of sterile bundles in memory and records a bundle in the ledger only
/// when it is assigned. When the process ends, the pool it held has no ledger owner and no holder,
/// and every start prepares a fresh pool beside it, so each restart leaked one pool. This runs
/// once at start, before the new pool is built.
///
/// A bundle is touched only when all of these hold:
///
/// - its name is exactly eight lowercase hex digits, the broker's own naming;
/// - no ledger record, assigned or released, names it (so it was never assigned, or its release
///   is already accounted for elsewhere);
/// - no process has its network namespace open, so no machine can be attached to it.
///
/// If the ledger or `/proc` cannot be read the function returns the error and removes nothing.
///
/// # Errors
///
/// Returns a ledger read failure, a kernel listing failure, or a failure to inspect `/proc`.
pub fn reap_unassigned(broker: &Broker) -> Result<ReapReport, Error> {
    let report = reconcile(broker)?;
    let mut ids = unowned_ids(&report);
    let mut outcome = ReapReport::default();
    if ids.is_empty() {
        return Ok(outcome);
    }
    let in_use = namespaces_in_use()?;
    while let Some(id) = ids.pop_first() {
        let pin = broker.namespace_dir().join(&id);
        if pin_in_use(&pin, &in_use) {
            outcome.in_use.push(id);
            continue;
        }
        if remove_unassigned(&BundleNames::new(&id), &pin) {
            outcome.removed.push(id);
        } else {
            outcome.failed.push(id);
        }
    }
    Ok(outcome)
}

/// The bundle names that appear in the kernel without a ledger owner, in the broker's naming.
fn unowned_ids(report: &ReconcileReport) -> BTreeSet<String> {
    let eight_hex = |text: &str| {
        text.len() == 8
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let mut ids = BTreeSet::new();
    for name in &report.unowned_namespaces {
        if eight_hex(name) {
            ids.insert(name.clone());
        }
    }
    for name in &report.unowned_links {
        if let Some(rest) = name.strip_prefix("sv").filter(|rest| eight_hex(rest)) {
            ids.insert(rest.to_owned());
        }
    }
    for name in &report.unowned_tables {
        for prefix in ["somah_", "somap_", "soma_"] {
            if let Some(rest) = name.strip_prefix(prefix).filter(|rest| eight_hex(rest)) {
                ids.insert(rest.to_owned());
            }
        }
    }
    ids
}

/// The `(device, inode)` of the network namespace of every process.
fn namespaces_in_use() -> Result<BTreeSet<(u64, u64)>, Error> {
    let mut used = BTreeSet::new();
    for entry in fs::read_dir("/proc").map_err(|error| Error::io(Step::OpenNamespace, &error))? {
        let entry = entry.map_err(|error| Error::io(Step::OpenNamespace, &error))?;
        if !entry
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }
        // A process that exits while this runs has no namespace to protect; a process whose
        // namespace cannot be read is not skipped silently.
        match fs::metadata(entry.path().join("ns/net")) {
            Ok(meta) => {
                used.insert((meta.dev(), meta.ino()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(Step::OpenNamespace, &error)),
        }
    }
    Ok(used)
}

fn pin_in_use(pin: &Path, used: &BTreeSet<(u64, u64)>) -> bool {
    fs::metadata(pin).is_ok_and(|meta| used.contains(&(meta.dev(), meta.ino())))
}

/// Removes one bundle's host objects and reports whether all of them are gone afterwards.
fn remove_unassigned(names: &BundleNames, pin: &Path) -> bool {
    // The veth goes first so nothing can cross the bundle while the rest is removed. Deleting the
    // host side removes its peer in the namespace. Conntrack is not flushed: forwarding stays off
    // until activation, so an unassigned bundle has carried no connection to forget.
    let steps = [
        netlink::delete_link(&names.host_veth).is_ok(),
        nft::delete_table(&names.publication_table).is_ok(),
        NetNamespace::unpin(pin).is_ok(),
        nft::delete_table(&names.host_table).is_ok(),
    ];
    steps.iter().all(|ok| *ok)
        && !pin.exists()
        && link::list_links().is_ok_and(|links| !links.contains(&names.host_veth))
        && nft::table_exists(&names.host_table).is_ok_and(|exists| !exists)
        && nft::table_exists(&names.publication_table).is_ok_and(|exists| !exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unowned_ids_takes_only_the_brokers_own_eight_hex_names() {
        let report = ReconcileReport {
            unowned_namespaces: vec![
                "0037835f".to_owned(),
                "keep-me".to_owned(),
                "0037835F".to_owned(),
            ],
            unowned_links: vec![
                "sv00a9afa0".to_owned(),
                "svXYZ".to_owned(),
                "eth0".to_owned(),
            ],
            unowned_tables: vec![
                "somah_0037835f".to_owned(),
                "somap_deadbeef".to_owned(),
                "miosa_fleet_dist".to_owned(),
                "somah_toolongname".to_owned(),
            ],
            ..ReconcileReport::default()
        };
        let ids: Vec<_> = unowned_ids(&report).into_iter().collect();
        assert_eq!(ids, ["0037835f", "00a9afa0", "deadbeef"]);
    }

    #[test]
    fn counts_partition_dispositions() {
        let bundle = BundleId::new([1; 16]).expect("id");
        let generation = CleanupGeneration::new(1).expect("g");
        let report = ReconcileReport {
            entries: vec![
                (bundle, generation, Disposition::Consistent),
                (bundle, generation, Disposition::Released),
                (bundle, generation, Disposition::Drifted(Drift::TapMissing)),
                (bundle, generation, Disposition::Orphaned),
            ],
            unowned_namespaces: vec!["x".to_owned()],
            unowned_links: Vec::new(),
            unowned_tables: vec!["soma_deadbeef".to_owned()],
        };
        assert_eq!(report.counts(), (2, 1, 1));
        assert_eq!(report.unowned(), 2);
    }
}
