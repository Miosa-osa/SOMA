//! Whether an attestation describes the jail the worker must be inside.
//!
//! The worker checks this itself instead of trusting the launcher, because a VMM that would
//! serve requests outside its jail is exactly the process the jail exists to prevent. Every
//! property below is one the jail profile guarantees, so a report that fails any of them means
//! the process is somewhere else and must not run.

use soma_jail::ProbeReport;

/// True only when the report proves a sealed descriptor table, an unprivileged identity in a
/// private PID namespace, and an empty read-only root without procfs or sysfs.
pub fn admits_service(report: &ProbeReport) -> bool {
    let identities = [report.uid, report.euid, report.gid, report.egid];
    report.table_sealed
        && report.first_bad_slot.is_none()
        && report.pid == 1
        && identities.iter().all(|identity| *identity != 0)
        && report.root.entries == 0
        && !report.root.writable
        && !report.root.proc_visible
        && !report.root.sys_visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use soma_jail::RootView;

    fn jailed() -> ProbeReport {
        ProbeReport {
            pid: 1,
            uid: 60_001,
            euid: 60_001,
            gid: 60_001,
            egid: 60_001,
            table_sealed: true,
            first_bad_slot: None,
            root: RootView {
                entries: 0,
                writable: false,
                proc_visible: false,
                sys_visible: false,
            },
        }
    }

    #[test]
    fn a_complete_jail_attestation_admits_service() {
        assert!(admits_service(&jailed()));
    }

    #[test]
    fn every_missing_jail_property_refuses_service() {
        let unsealed = ProbeReport {
            table_sealed: false,
            first_bad_slot: Some(4),
            ..jailed()
        };
        let host_pid = ProbeReport {
            pid: 4_242,
            ..jailed()
        };
        let root_identity = ProbeReport { uid: 0, ..jailed() };
        let root_group = ProbeReport {
            egid: 0,
            ..jailed()
        };
        let populated_root = ProbeReport {
            root: RootView {
                entries: 3,
                ..jailed().root
            },
            ..jailed()
        };
        let writable_root = ProbeReport {
            root: RootView {
                writable: true,
                ..jailed().root
            },
            ..jailed()
        };
        let with_procfs = ProbeReport {
            root: RootView {
                proc_visible: true,
                ..jailed().root
            },
            ..jailed()
        };
        let with_sysfs = ProbeReport {
            root: RootView {
                sys_visible: true,
                ..jailed().root
            },
            ..jailed()
        };
        let refused = [
            unsealed,
            host_pid,
            root_identity,
            root_group,
            populated_root,
            writable_root,
            with_procfs,
            with_sysfs,
        ];
        for report in refused {
            assert!(!admits_service(&report), "{report:?}");
        }
    }
}
