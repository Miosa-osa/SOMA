//! Reconciliation of the durable ledger against kernel reality.
//!
//! Every ledger entry is compared with the namespace pin, the host veth, and the host table it
//! should own. [`reconcile`] only reports: kernel objects carrying SOMA names without a ledger
//! owner are listed and never removed by it, because ownership must be proven before cleanup.
//! [`reap_unassigned`] is the one place that removes them, and only for a bundle that has a pin in
//! this broker's own directory, is named by no ledger record, and is held by no process.

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
    reconcile_with(broker, &|_, _| false)
}

/// Compares the ledger with the kernel, including what the sandbox ruleset of a bundle claimed
/// with no egress actually lets through.
///
/// `window_open` says whether the caller has recorded an opened egress for a bundle. A bundle
/// whose base intent denies egress, whose live ruleset lets connections out, and for which no
/// window is recorded is drifted: egress is open and nobody asked for it.
///
/// # Errors
///
/// Returns a ledger read failure or a kernel listing failure.
pub fn reconcile_with(
    broker: &Broker,
    window_open: &dyn Fn(BundleId, CleanupGeneration) -> bool,
) -> Result<ReconcileReport, Error> {
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
        } else if base_is_breached(broker, &entry.record, &names, window_open) {
            Disposition::Drifted(Drift::RulesetNotBase)
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

/// Whether the live ruleset of a bundle claimed with no egress lets connections out although no
/// window is recorded for it.
///
/// A bundle whose base intent forwards is not examined: its ruleset is what it was claimed with.
/// One whose ruleset cannot be read is breached too, because "could not tell" is not "closed".
fn base_is_breached(
    broker: &Broker,
    record: &crate::AssignmentRecord,
    names: &BundleNames,
    window_open: &dyn Fn(BundleId, CleanupGeneration) -> bool,
) -> bool {
    if record.intent.egress().forwards() {
        return false;
    }
    let pin = broker.namespace_dir().join(record.bundle.short_hex());
    let live = NetNamespace::open(&pin).and_then(|namespace| {
        namespace.within(|| {
            let text = nft::chain_text(&names.sandbox_table, "forward")?;
            Ok(nft::chain_forwards(&text, &names.tap))
        })
    });
    match live {
        Ok(open) => open && !window_open(record.bundle, record.generation),
        Err(_) => true,
    }
}

/// What [`reap_unassigned`] did with the kernel objects of bundles no ledger record owns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReapReport {
    /// Bundle names whose objects were all removed and verified absent.
    pub removed: Vec<String>,
    /// Bundle names left alone because a process still has their network namespace open.
    pub in_use: Vec<String>,
    /// Bundle names planned for removal beyond [`MAX_REAP_PER_START`]; the next start takes them.
    pub deferred: Vec<String>,
    /// Bundle names where a removal step failed, an object remained afterwards, or the pin could
    /// not be inspected. Nothing is removed for a pin that could not be inspected.
    pub failed: Vec<String>,
    /// Names seen in the kernel with the broker's naming but with no namespace pin in this
    /// broker's directory. They may belong to another broker, so they are reported and never
    /// touched.
    pub foreign: Vec<String>,
}

/// Removes the sterile bundles a previous broker process prepared and never assigned.
///
/// A broker prepares a pool of sterile bundles in memory and records a bundle in the ledger only
/// when it is assigned. When the process ends, the pool it held has no ledger owner and no holder,
/// and every start prepared a fresh pool beside it, so each restart leaked one pool. This runs
/// once at start, before the new pool is built.
///
/// A bundle is touched only when all of these hold:
///
/// - it has a namespace pin in this broker's own pin directory, which is what makes it ours:
///   links and tables are host-wide, so a name seen only there is never enough;
/// - the pin's name is exactly eight lowercase hex digits, the broker's own naming;
/// - no ledger record, assigned or released, names it (so it was never assigned, or its release
///   is already accounted for elsewhere);
/// - no process has its network namespace open, so no machine can be attached to it, and the pin
///   could be inspected to find that out.
///
/// If the ledger or `/proc` cannot be read the function returns the error and removes nothing.
///
/// # Errors
///
/// Returns a ledger read failure, a kernel listing failure, or a failure to inspect `/proc`.
pub fn reap_unassigned(broker: &Broker) -> Result<ReapReport, Error> {
    Ok(reap_plan(broker)?.execute(broker, usize::MAX))
}

/// The most bundles one start removes. A larger backlog is finished by the next start, so a start
/// is never held up by a pool left behind by many restarts.
pub const MAX_REAP_PER_START: usize = 1024;

/// What a start decided to reap, computed before any removal.
///
/// The decision is taken once, before the new pool exists, so removal can run beside the
/// listener without ever being able to name a bundle this process built.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReapPlan {
    free: Vec<String>,
    outcome: ReapReport,
}

impl ReapPlan {
    /// The bundles that will be removed.
    #[must_use]
    pub fn candidates(&self) -> &[String] {
        &self.free
    }

    /// Removes up to `limit` of the planned bundles and returns the full report, including what
    /// the plan already knew (in use, unreadable, foreign) and what was deferred past `limit`.
    #[must_use]
    pub fn execute(self, broker: &Broker, limit: usize) -> ReapReport {
        let Self { free, mut outcome } = self;
        for (index, id) in free.into_iter().enumerate() {
            if index >= limit {
                outcome.deferred.push(id);
                continue;
            }
            let pin = broker.namespace_dir().join(&id);
            if remove_unassigned(&BundleNames::new(&id), &pin) {
                outcome.removed.push(id);
            } else {
                outcome.failed.push(id);
            }
        }
        outcome
    }
}

/// Decides which unassigned bundles to reap, without removing anything.
///
/// # Errors
///
/// Returns a ledger read failure, a kernel listing failure, a `/proc` that does not show every
/// process, or a failure to inspect `/proc`. In each case nothing is planned.
pub fn reap_plan(broker: &Broker) -> Result<ReapPlan, Error> {
    let report = reconcile(broker)?;
    let (ours, foreign) = split_by_pin(&report);
    let mut plan = ReapPlan {
        outcome: ReapReport {
            foreign: foreign.into_iter().collect(),
            ..ReapReport::default()
        },
        ..ReapPlan::default()
    };
    if ours.is_empty() {
        return Ok(plan);
    }
    // Whether a process holds a namespace is learned from /proc, so a /proc that hides other
    // processes would make every bundle look free.
    require_complete_proc()?;
    let in_use = namespaces_in_use()?;
    for id in ours {
        let pin = broker.namespace_dir().join(&id);
        match pin_state(fs::metadata(&pin), &in_use) {
            PinState::InUse => plan.outcome.in_use.push(id),
            PinState::Gone => {}
            PinState::Unreadable => plan.outcome.failed.push(id),
            PinState::Free => plan.free.push(id),
        }
    }
    Ok(plan)
}

/// Fails unless `/proc` shows every process.
///
/// `hidepid=1`, `hidepid=2` (also spelt `noaccess` and `invisible`), and a unit's `ProtectProc=`
/// all remove other processes from the listing without an error, which would make a held
/// namespace look unheld.
fn require_complete_proc() -> Result<(), Error> {
    let mounts = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| Error::io(Step::OpenNamespace, &error))?;
    if proc_hides_processes(&mounts) {
        return Err(Error::Kernel {
            step: Step::OpenNamespace,
            errno: libc::EPERM,
        });
    }
    Ok(())
}

/// Whether the `/proc` mount in `mountinfo` carries a `hidepid` setting that hides processes.
fn proc_hides_processes(mountinfo: &str) -> bool {
    mountinfo.lines().any(|line| {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(dash) = fields.iter().position(|field| *field == "-") else {
            return false;
        };
        let is_proc = fields.get(4) == Some(&"/proc") && fields.get(dash + 1) == Some(&"proc");
        is_proc
            && line
                .split([' ', ','])
                .filter_map(|option| option.strip_prefix("hidepid="))
                .any(|value| !matches!(value, "0" | "off"))
    })
}

fn eight_hex(text: &str) -> bool {
    text.len() == 8
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Splits the names the kernel shows without a ledger owner into those with a pin in our own
/// directory (ours to reap) and those seen only as a host-wide link or table (never touched).
fn split_by_pin(report: &ReconcileReport) -> (BTreeSet<String>, BTreeSet<String>) {
    let ours: BTreeSet<String> = report
        .unowned_namespaces
        .iter()
        .filter(|name| eight_hex(name))
        .cloned()
        .collect();
    let mut foreign = BTreeSet::new();
    for name in &report.unowned_links {
        if let Some(rest) = name.strip_prefix("sv").filter(|rest| eight_hex(rest)) {
            foreign.insert(rest.to_owned());
        }
    }
    for name in &report.unowned_tables {
        for prefix in ["somah_", "somap_", "soma_"] {
            if let Some(rest) = name.strip_prefix(prefix).filter(|rest| eight_hex(rest)) {
                foreign.insert(rest.to_owned());
            }
        }
    }
    let foreign = foreign.difference(&ours).cloned().collect();
    (ours, foreign)
}

/// What inspecting one pin concluded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PinState {
    /// A process has the namespace open.
    InUse,
    /// The pin is there and nothing has it open.
    Free,
    /// The pin disappeared between the listing and now, so there is nothing to remove.
    Gone,
    /// The pin could not be inspected; it is left alone.
    Unreadable,
}

fn pin_state(metadata: std::io::Result<fs::Metadata>, used: &BTreeSet<(u64, u64)>) -> PinState {
    match metadata {
        Ok(meta) if used.contains(&(meta.dev(), meta.ino())) => PinState::InUse,
        Ok(_) => PinState::Free,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PinState::Gone,
        Err(_) => PinState::Unreadable,
    }
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

/// Removes one bundle's host objects and reports whether all of them are gone afterwards.
fn remove_unassigned(names: &BundleNames, pin: &Path) -> bool {
    // The veth goes first so nothing can cross the bundle while the rest is removed; deleting the
    // host side removes its peer in the namespace. The pin goes LAST: it is what makes the bundle
    // ours, so a failure part way leaves the pin, the bundle is still ours at the next start, and
    // whatever remains is retried. Unpinning earlier would leave a stray veth or table that nothing
    // of ours could claim again. Conntrack is not flushed: forwarding stays off until activation,
    // so an unassigned bundle has carried no connection to forget.
    let steps = [
        netlink::delete_link(&names.host_veth).is_ok(),
        nft::delete_table(&names.publication_table).is_ok(),
        nft::delete_table(&names.host_table).is_ok(),
    ];
    let objects_gone = steps.iter().all(|ok| *ok)
        && link::list_links().is_ok_and(|links| !links.contains(&names.host_veth))
        && nft::table_exists(&names.host_table).is_ok_and(|exists| !exists)
        && nft::table_exists(&names.publication_table).is_ok_and(|exists| !exists);
    objects_gone && NetNamespace::unpin(pin).is_ok() && !pin.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pin_in_our_directory_makes_a_bundle_ours() {
        let report = ReconcileReport {
            unowned_namespaces: vec![
                "0037835f".to_owned(),
                "keep-me".to_owned(),
                "0037835F".to_owned(),
            ],
            unowned_links: vec![
                "sv00a9afa0".to_owned(),
                "sv0037835f".to_owned(),
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
        let (ours, foreign) = split_by_pin(&report);
        assert_eq!(ours.into_iter().collect::<Vec<_>>(), ["0037835f"]);
        assert_eq!(
            foreign.into_iter().collect::<Vec<_>>(),
            ["00a9afa0", "deadbeef"],
            "a link or table with no pin of ours is reported, never reaped"
        );
    }

    #[test]
    fn a_pin_that_cannot_be_inspected_is_left_alone() {
        use std::io::{Error as IoError, ErrorKind};
        let used = BTreeSet::new();
        assert_eq!(
            pin_state(Err(IoError::from(ErrorKind::PermissionDenied)), &used),
            PinState::Unreadable
        );
        assert_eq!(
            pin_state(Err(IoError::from(ErrorKind::Other)), &used),
            PinState::Unreadable
        );
        assert_eq!(
            pin_state(Err(IoError::from(ErrorKind::NotFound)), &used),
            PinState::Gone
        );
        let here = fs::metadata("/proc/self/ns/net").expect("own namespace");
        let mut used = BTreeSet::new();
        used.insert((here.dev(), here.ino()));
        assert_eq!(pin_state(Ok(here), &used), PinState::InUse);
    }

    #[test]
    fn a_proc_that_hides_processes_is_detected() {
        let plain = "26 25 0:5 / /proc rw,nosuid - proc proc rw\n";
        let hidden = "26 25 0:5 / /proc rw,nosuid,hidepid=2,gid=100 - proc proc rw\n";
        let named = "26 25 0:5 / /proc rw,nosuid - proc proc rw,hidepid=invisible\n";
        let off = "26 25 0:5 / /proc rw,nosuid - proc proc rw,hidepid=off\n";
        let other = "30 25 0:40 / /srv rw - xfs /dev/x rw,hidepid=2\n";
        assert!(!proc_hides_processes(plain));
        assert!(proc_hides_processes(hidden));
        assert!(proc_hides_processes(named));
        assert!(!proc_hides_processes(off));
        assert!(!proc_hides_processes(other), "only the /proc mount counts");
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
