//! Whether this host can build a jail at all, and where.
//!
//! Building a jail means creating a user namespace with an identity map, a mount namespace with
//! an empty root, and a cgroup v2 leaf. A host that cannot do those things cannot hold a machine
//! inside one, and the only honest answer is to refuse the launch: a broker that quietly fell
//! back to holding the machine itself would reopen exactly the gap the jail exists to close.

use std::path::PathBuf;

use soma::BackendFailureKind;
use soma_jail::HostAnchors;

/// Names the delegated cgroup2 subtree the jail creates its leaf under.
const CGROUP_ROOT: &str = "SOMA_JAIL_CGROUP_ROOT";
/// Names where the empty jail roots are created, one per leaf.
const JAIL_ROOT_PARENT: &str = "SOMA_JAIL_ROOT_PARENT";
/// Names the statically linked worker the launcher executes inside the jail.
///
/// The jail root is empty, so a dynamically linked worker has no loader to start it. The
/// binary is named rather than found, because a broker that searched for it could execute
/// something other than the worker this host was installed with.
const WORKER_BINARY: &str = "SOMA_VMM_BINARY";
/// Names where the jailed workers' standard streams go; unset means nowhere.
const LOG_DIRECTORY: &str = "SOMA_JAIL_LOG_DIR";

/// Where this host may build jails, and what it runs inside them.
pub(crate) struct Anchors {
    pub(super) host: HostAnchors,
    pub(super) worker: PathBuf,
    log_directory: Option<PathBuf>,
}

impl Anchors {
    /// Reads the operator's jail configuration.
    ///
    /// # Errors
    ///
    /// Returns [`BackendFailureKind::Unsupported`] when this host was not configured to build
    /// jails, and [`BackendFailureKind::Unavailable`] when it was but the configuration does
    /// not describe a usable one.
    pub(crate) fn configured() -> Result<Self, BackendFailureKind> {
        let cgroup_root = directory(CGROUP_ROOT)?;
        let jail_root_parent = directory(JAIL_ROOT_PARENT)?;
        let worker = PathBuf::from(named(WORKER_BINARY)?);
        if !worker.is_file() {
            return Err(BackendFailureKind::Unavailable);
        }
        std::fs::create_dir_all(&jail_root_parent).map_err(|_| BackendFailureKind::Unavailable)?;
        Ok(Self {
            host: HostAnchors {
                cgroup_root,
                jail_root_parent,
            },
            worker,
            log_directory: std::env::var_os(LOG_DIRECTORY).map(PathBuf::from),
        })
    }

    /// Whether this host was configured to jail its machines at all.
    pub(crate) fn is_configured() -> bool {
        [CGROUP_ROOT, JAIL_ROOT_PARENT, WORKER_BINARY]
            .into_iter()
            .all(|name| std::env::var_os(name).is_some())
    }

    /// Records one line about a jailed worker beside its own output.
    ///
    /// A jailed machine has no name, no socket, and no path anything can address it by, so the
    /// only way an operator can find the process holding one Instance is if the broker that
    /// built it says where it is and what it attested.
    pub(super) fn record(&self, instance: &str, line: &str) {
        use std::io::Write as _;

        let Some(directory) = self.log_directory.as_ref() else {
            return;
        };
        let path = directory.join(format!("{instance}.log"));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
        {
            let _ignored = writeln!(file, "{line}");
        }
    }

    /// Where one worker's standard output and error go.
    pub(super) fn log_for(&self, instance: &str) -> PathBuf {
        self.log_directory.as_ref().map_or_else(
            || PathBuf::from("/dev/null"),
            |directory| directory.join(format!("{instance}.log")),
        )
    }
}

fn named(variable: &str) -> Result<std::ffi::OsString, BackendFailureKind> {
    std::env::var_os(variable).ok_or(BackendFailureKind::Unsupported)
}

fn directory(variable: &str) -> Result<PathBuf, BackendFailureKind> {
    let path = PathBuf::from(named(variable)?);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(BackendFailureKind::Unavailable)
    }
}
