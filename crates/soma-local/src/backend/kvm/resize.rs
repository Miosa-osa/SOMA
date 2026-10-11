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
const CGROUP_ROOT: &str = "SOMA_JAIL_CGROUP_ROOT";

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
            None => memory::report_unchanged(live.memory_mib, live.memory_mib, 0),
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
    let leaf = CgroupLeaf::create_cpu_only(root, &format!("{name}-live"))
        .map_err(|_| "cpu_controller_not_delegated")?;
    leaf.attach(std::process::id())
        .map_err(|_| "cannot_join_cpu_leaf")?;
    Ok(leaf)
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
    fn a_machine_can_be_given_between_one_vcpu_and_the_count_it_booted_with() {
        assert_eq!(plan_cpu(4, 4, 2), CpuPlan::Apply);
        assert_eq!(plan_cpu(4, 2, 4), CpuPlan::Apply);
        assert_eq!(plan_cpu(4, 2, 2), CpuPlan::Same);
        assert_eq!(plan_cpu(4, 4, 5), CpuPlan::Reject("outside_booted_vcpus"));
        assert_eq!(plan_cpu(4, 4, 0), CpuPlan::Reject("outside_booted_vcpus"));
        assert_eq!(plan_cpu(1, 1, 1), CpuPlan::Same);
    }
}
