//! Shared fixture for the privileged live tests: prerequisites, specifications, resources,
//! and residue checks.
//!
//! Every prerequisite failure is explicit; nothing here skips.

#![allow(unsafe_code)]

use std::{
    env, fs, io,
    os::fd::{FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use soma_jail::{
    CgroupLimits, CpuMax, DescriptorManifest, DescriptorRole, HostAnchors, Identity, JailSpec,
    LeafName, LedgerRecord, Phase, Resources, Rlimits, launch,
};

use super::control::{Control, Jail};

pub(crate) const IDENTITY: Identity = Identity {
    uid: 60_001,
    gid: 60_001,
};
pub(crate) const MEMORY_MAX: u64 = 64 << 20;
pub(crate) const PIDS_MAX: u32 = 8;
pub(crate) const NOFILE: u32 = 16;
pub(crate) const ROLES: [DescriptorRole; 1] = [DescriptorRole::Control];
pub(crate) const KVM_ROLES: [DescriptorRole; 2] = [DescriptorRole::Kvm, DescriptorRole::Control];

/// Where the live run may create leaves and jail roots, and which probe to execute.
pub(crate) struct Live {
    pub cgroup_root: PathBuf,
    pub jail_root_parent: PathBuf,
    pub probe: PathBuf,
    pub kvm: bool,
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Fails with the complete list of unmet prerequisites; never skips.
pub(crate) fn require() -> Live {
    let mut missing = Vec::new();
    // SAFETY: `geteuid` takes no arguments and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        missing.push("root inside the privileged container (scripts/jail-live-tests.sh)".into());
    }
    let cgroup_root = env::var_os("SOMA_JAIL_CGROUP_ROOT").map(PathBuf::from);
    match &cgroup_root {
        None => missing.push("SOMA_JAIL_CGROUP_ROOT (a delegated cgroup2 subtree)".into()),
        Some(root) => {
            let delegated = fs::read_to_string(root.join("cgroup.subtree_control"));
            for controller in ["cpu", "memory", "pids"] {
                let present = delegated
                    .as_deref()
                    .is_ok_and(|list| list.split_whitespace().any(|found| found == controller));
                if !present {
                    missing.push(format!("{controller} delegated under {}", root.display()));
                }
            }
        }
    }
    let probe = env::var_os("SOMA_JAIL_PROBE").map(PathBuf::from);
    match &probe {
        Some(path) if path.is_file() => {}
        Some(path) => missing.push(format!("SOMA_JAIL_PROBE file {}", path.display())),
        None => missing.push("SOMA_JAIL_PROBE (path to the static jail-probe)".into()),
    }
    let jail_root_parent = env::var_os("SOMA_JAIL_ROOT_PARENT")
        .map_or_else(|| PathBuf::from("/tmp/soma-jail-live"), PathBuf::from);
    if let Err(error) = fs::create_dir_all(&jail_root_parent) {
        missing.push(format!(
            "jail root parent {}: {error}",
            jail_root_parent.display()
        ));
    }
    assert!(
        missing.is_empty(),
        "live jail tests need: {}",
        missing.join("; ")
    );
    let kvm = Path::new("/dev/kvm").exists();
    let cgroup_root = cgroup_root.expect("checked");
    Live {
        cgroup_root,
        jail_root_parent,
        probe: probe.expect("checked"),
        kvm,
    }
}

impl Live {
    pub(crate) fn anchors(&self) -> HostAnchors {
        HostAnchors {
            cgroup_root: self.cgroup_root.clone(),
            jail_root_parent: self.jail_root_parent.clone(),
        }
    }

    pub(crate) fn require_kvm(&self) {
        assert!(
            self.kvm,
            "/dev/kvm is absent; pass the host device into the container"
        );
    }

    /// Opens the null stream, an unlinked log file, the probe, and every role in order.
    pub(crate) fn resources(&self, roles: &[DescriptorRole]) -> (Resources, Control) {
        let null = fs::File::open("/dev/null").expect("/dev/null").into();
        let log_path = self
            .jail_root_parent
            .join(format!("log-{}", std::process::id()));
        let log = fs::File::create(&log_path).expect("log file").into();
        let _ = fs::remove_file(&log_path);
        let executable = fs::File::open(&self.probe)
            .expect("probe executable")
            .into();
        let (parent, child) = seqpacket_pair();
        let mut child = Some(child);
        let descriptors = roles
            .iter()
            .map(|role| {
                let fd: OwnedFd = match role {
                    DescriptorRole::Kvm => {
                        let kvm = fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open("/dev/kvm");
                        kvm.expect("/dev/kvm").into()
                    }
                    DescriptorRole::Control => child.take().expect("one control socket"),
                    other => panic!("live tests do not provide {other:?}"),
                };
                (*role, fd)
            })
            .collect();
        (
            Resources {
                null,
                log,
                executable,
                descriptors,
            },
            Control(parent),
        )
    }

    /// Launches the probe and fails loudly with the typed failure and cleanup disposition.
    pub(crate) fn launch(
        &self,
        name: &str,
        roles: &[DescriptorRole],
        limits: CgroupLimits,
    ) -> Jail {
        let (resources, control) = self.resources(roles);
        let spec = spec(name, roles, limits);
        match launch(&spec, &self.anchors(), resources) {
            Ok(handle) => Jail { handle, control },
            Err(failure) => panic!("launch {name}: {failure}"),
        }
    }

    /// Proves the leaf, the jail root, and the process named by `record` are gone.
    pub(crate) fn assert_zero_residual(&self, record: &LedgerRecord) {
        let leaf = self.cgroup_root.join(&record.leaf);
        assert!(!leaf.exists(), "cgroup leaf {} survived", leaf.display());
        assert!(
            !record.jail_root.exists(),
            "jail root {} survived",
            record.jail_root.display()
        );
        if let Some(pid) = record.pid {
            let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
            assert!(
                !status.contains("jail-probe"),
                "process {pid} survived:\n{status}"
            );
        }
    }
}

pub(crate) fn leaf(name: &str) -> LeafName {
    LeafName::new(&format!("live-{name}-{}", std::process::id())).expect("leaf name")
}

pub(crate) fn limits() -> CgroupLimits {
    CgroupLimits {
        memory_max_bytes: MEMORY_MAX,
        cpu_max: CpuMax {
            quota_us: 100_000,
            period_us: 100_000,
        },
        pids_max: PIDS_MAX,
        io_max: None,
    }
}

pub(crate) fn spec(name: &str, roles: &[DescriptorRole], limits: CgroupLimits) -> JailSpec {
    let rlimits = Rlimits {
        nofile: NOFILE,
        nproc: 64,
        fsize_bytes: 1 << 30,
        address_space_bytes: None,
    };
    JailSpec {
        identity: IDENTITY,
        leaf: leaf(name),
        limits,
        rlimits,
        manifest: DescriptorManifest::new(roles.to_vec()).expect("manifest"),
        phase: Phase::Startup,
    }
}

pub(crate) fn seqpacket_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0 as libc::c_int; 2];
    let kind = libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC;
    // SAFETY: `fds` is valid storage for two descriptors.
    let result = unsafe { libc::socketpair(libc::AF_UNIX, kind, 0, fds.as_mut_ptr()) };
    assert_eq!(result, 0, "socketpair failed: errno {}", errno());
    // SAFETY: both descriptors were just created and nothing else owns them.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

pub(crate) fn deadline(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

/// Open descriptor numbers of `pid`, read through the launcher's procfs.
pub(crate) fn open_fds(pid: i32) -> Vec<u32> {
    let mut fds: Vec<u32> = fs::read_dir(format!("/proc/{pid}/fd"))
        .expect("child fd table")
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    fds.sort_unstable();
    fds
}
