//! Live resize of the machine this process holds.
//!
//! CPU is a share: the cgroup that holds the machine's threads is told how much CPU time it may
//! use per period, up to the vCPUs the machine booted with. The guest keeps seeing the cores it
//! booted with, and the host stops granting more time than the share, so the number reported is
//! the cap the kernel read back and nothing else.

use std::path::PathBuf;

use soma::InstanceId;
use soma_jail::{CgroupLeaf, CpuMax};

use super::{KvmBackend, held::Held};

mod memory;
use crate::{CpuResize, DimensionState, ResizeFailure, ResizeOutcome, ResizeRequest};

/// The accounting period every live share is expressed against.
const PERIOD_US: u64 = 100_000;
/// Names the delegated cgroup2 subtree live limits are created under for an unjailed host.
pub(super) const CGROUP_ROOT: &str = "SOMA_JAIL_CGROUP_ROOT";

impl KvmBackend {
    /// Applies `request` to the sandbox this process is driving.
    pub(super) fn resize_resident(
        &mut self,
        instance: &InstanceId,
        request: ResizeRequest,
    ) -> Result<ResizeOutcome, ResizeFailure> {
        let Some(live) = self.live_for(instance) else {
            return Err(ResizeFailure::NotFound);
        };
        let jailed = matches!(live.held, Held::Jailed(_));
        let cpu = match request.cpu_count {
            None => report(
                live.vcpus,
                live.cpu_limit,
                None,
                DimensionState::Unchanged,
                None,
            ),
            Some(wanted) => resize_cpu(instance, live, wanted, jailed),
        };
        let memory = match request.memory_mb {
            None => match &mut live.held {
                Held::Resident(session) => {
                    memory::current(&mut memory::ResidentLink::new(session), live.memory_mib)
                }
                Held::Jailed(_) => memory::report_unchanged(live.memory_mib, live.memory_mib, 0),
            },
            Some(wanted) => match &mut live.held {
                Held::Resident(session) => memory::resize(
                    &mut memory::ResidentLink::new(session),
                    live.memory_mib,
                    wanted,
                    &memory::REAL,
                    &mut std::thread::sleep,
                ),
                Held::Jailed(_) => memory::unsupported(
                    live.memory_mib,
                    request.memory_mb,
                    "jailed_machines_do_not_resize_memory_yet",
                ),
            },
        };
        Ok(ResizeOutcome {
            host_pid: std::process::id(),
            cpu,
            memory,
        })
    }
}

impl KvmBackend {
    /// Brings a machine that was just launched to the limits its request named.
    ///
    /// A Generation boots at its own vCPU count and memory. A smaller vCPU request is a CPU
    /// quota and a larger memory request is plugged memory, both applied before the Launch is
    /// answered so the shape it reports is the shape the machine has.
    pub(super) fn fit_to_request(
        &mut self,
        instance: &InstanceId,
        shape: &soma::MachineShape,
    ) -> Result<(), soma::BackendFailureKind> {
        use soma::BackendFailureKind::Unavailable;
        let Some(live) = self.live_for(instance) else {
            return Err(Unavailable);
        };
        let jailed = matches!(live.held, Held::Jailed(_));
        if shape.vcpu_count() != live.vcpus {
            let applied = resize_cpu(instance, live, shape.vcpu_count(), jailed);
            if applied.state != DimensionState::AppliedLive {
                return Err(Unavailable);
            }
        }
        if shape.memory_mib() > live.memory_mib {
            let Held::Resident(session) = &mut live.held else {
                return Err(Unavailable);
            };
            let plugged = memory::resize(
                &mut memory::ResidentLink::new(session),
                live.memory_mib,
                shape.memory_mib(),
                &memory::REAL,
                &mut std::thread::sleep,
            );
            if plugged.state != DimensionState::AppliedLive
                || plugged.total_mb != shape.memory_mib()
            {
                return Err(Unavailable);
            }
        }
        Ok(())
    }
}

/// What a requested vCPU count means for a machine.
#[derive(Debug, Eq, PartialEq)]
enum CpuPlan {
    Reject(&'static str),
    Same,
    Apply,
}

/// Decides, before anything is written, whether `wanted` vCPUs can be given.
fn plan_cpu(booted: u16, current: u16, wanted: u16) -> CpuPlan {
    if wanted == 0 || wanted > booted {
        CpuPlan::Reject("outside_booted_vcpus")
    } else if wanted == current {
        CpuPlan::Same
    } else {
        CpuPlan::Apply
    }
}

fn resize_cpu(
    instance: &InstanceId,
    live: &mut super::lifecycle::Live,
    wanted: u16,
    jailed: bool,
) -> CpuResize {
    let booted = live.vcpus;
    let say =
        |state, reason: &str| report(booted, live.cpu_limit, Some(wanted), state, Some(reason));
    match plan_cpu(booted, live.cpu_limit, wanted) {
        CpuPlan::Reject(reason) => return say(DimensionState::Unsupported, reason),
        CpuPlan::Same => {
            return report(
                booted,
                live.cpu_limit,
                Some(wanted),
                DimensionState::Unchanged,
                None,
            );
        }
        CpuPlan::Apply => {}
    }
    let Some(root) = std::env::var_os(CGROUP_ROOT).map(PathBuf::from) else {
        return say(DimensionState::Unsupported, "no_cgroup_root");
    };
    let leaf = match leaf_for(&root, instance, jailed) {
        Ok(leaf) => leaf,
        Err(reason) => return say(DimensionState::Failed, reason),
    };
    let cap = CpuMax {
        quota_us: u64::from(wanted) * PERIOD_US,
        period_us: PERIOD_US,
    };
    match leaf.set_cpu_max(cap) {
        Ok(applied) if applied == cap => {
            live.cpu_limit = wanted;
            report(
                booted,
                wanted,
                Some(wanted),
                DimensionState::AppliedLive,
                None,
            )
        }
        Ok(_) => say(DimensionState::Failed, "kernel_reported_a_different_share"),
        Err(_) => say(DimensionState::Failed, "cgroup_write_failed"),
    }
}

/// The leaf that holds this machine's threads: the jail's own, or one made for an unjailed host.
fn leaf_for(
    root: &std::path::Path,
    instance: &InstanceId,
    jailed: bool,
) -> Result<CgroupLeaf, &'static str> {
    let name = format!("soma-{}", instance.as_str());
    if jailed {
        return CgroupLeaf::open_existing(root, &name).map_err(|_| "jail_leaf_missing");
    }
    sweep_finished_leaves(root);
    let leaf = CgroupLeaf::create_cpu_only(root, &format!("{name}-live"))
        .map_err(|_| "cpu_controller_not_delegated")?;
    leaf.attach(std::process::id())
        .map_err(|_| "cannot_join_cpu_leaf")?;
    Ok(leaf)
}

/// Removes the live-share leaves of machines that have ended.
///
/// A machine host joins its own leaf and exits without being able to remove it, so every sandbox that
/// ever ran with a CPU share would leave an empty directory behind. `remove_dir` on a cgroup succeeds
/// only when nothing runs in it, so this can never take a leaf from a live machine, and every launch
/// that makes a leaf clears the finished ones first.
pub(super) fn sweep_finished_leaves(root: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("soma-") && name.ends_with("-live") {
            let _ignored = std::fs::remove_dir(entry.path());
        }
    }
}

fn report(
    booted: u16,
    applied: u16,
    requested: Option<u16>,
    state: DimensionState,
    reason: Option<&str>,
) -> CpuResize {
    CpuResize {
        state,
        requested,
        applied,
        booted,
        reason: reason.map(ToOwned::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_leaves_are_swept_and_live_ones_and_other_names_are_kept() {
        let root = std::env::temp_dir().join(format!("soma-sweep-{}", std::process::id()));
        std::fs::create_dir_all(root.join("soma-aaa-live")).expect("finished leaf");
        std::fs::create_dir_all(root.join("soma-bbb-live")).expect("busy leaf");
        std::fs::write(root.join("soma-bbb-live").join("cgroup.procs"), "1\n").expect("member");
        std::fs::create_dir_all(root.join("other")).expect("other");
        sweep_finished_leaves(&root);
        assert!(
            !root.join("soma-aaa-live").exists(),
            "an empty leaf is removed"
        );
        assert!(
            root.join("soma-bbb-live").exists(),
            "a leaf with a member stays"
        );
        assert!(
            root.join("other").exists(),
            "a directory that is not a live leaf stays"
        );
        std::fs::remove_dir_all(&root).expect("clean up");
    }

    #[test]
    fn a_machine_can_be_given_between_one_vcpu_and_the_count_it_booted_with() {
        assert_eq!(plan_cpu(4, 4, 2), CpuPlan::Apply);
        assert_eq!(plan_cpu(4, 2, 4), CpuPlan::Apply);
        assert_eq!(plan_cpu(4, 2, 2), CpuPlan::Same);
        assert_eq!(plan_cpu(4, 4, 5), CpuPlan::Reject("outside_booted_vcpus"));
        assert_eq!(plan_cpu(4, 4, 0), CpuPlan::Reject("outside_booted_vcpus"));
        assert_eq!(plan_cpu(1, 1, 1), CpuPlan::Same);
    }
}
