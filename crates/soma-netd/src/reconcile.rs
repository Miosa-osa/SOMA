//! Reconciliation of the durable ledger against kernel reality.
//!
//! Every ledger entry is compared with the namespace pin, the host veth, and the host table it
//! should own; kernel objects carrying SOMA names without a ledger owner are reported but never
//! removed, because ownership must be proven before cleanup.

use crate::{
    Broker, BundleId, BundleNames, CleanupGeneration, Drift, Error, link, namespace::NetNamespace,
    nft,
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
        let disposition = if entry.released {
            if pin || veth || table {
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
        .filter(|name| name.starts_with("soma_") || name.starts_with("somah_"))
        .filter(|name| {
            !owned_short
                .iter()
                .any(|short| *name == format!("soma_{short}") || *name == format!("somah_{short}"))
        })
        .collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

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
