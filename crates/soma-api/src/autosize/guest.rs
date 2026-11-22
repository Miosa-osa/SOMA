//! Reading what a sandbox uses: from the host by default, from the guest when the host cannot say.
//!
//! The host reads the process that holds the machine: its CPU time and its resident memory, which
//! is the guest memory the machine has touched plus the monitor's own. That needs nothing from the
//! guest, so it works while a long command holds the guest's one session, which is exactly when a
//! build runs short of memory. The guest's own counters are the fallback for a machine whose holding
//! process this host cannot see.

use soma::{FileAnswer, FileMachineRequest, FileOperation, InstanceId, OperationId};

use crate::facade::SandboxFacade;

/// One reading: CPU counters (busy, total, host-or-guest) and memory used and held.
pub(super) struct Reading {
    pub(super) ticks: (u64, u64, bool),
    pub(super) memory_used_mb: u64,
    pub(super) memory_total_mb: u64,
}

/// Clock ticks per second in `/proc/<pid>/stat`; every Linux target SOMA runs on uses 100.
const CLOCK_TICKS: u64 = 100;
/// What the monitor process holds besides guest memory, left out of what the guest uses.
const MONITOR_OVERHEAD_MB: u64 = 48;

impl Reading {
    /// Processors kept busy since `previous`, in whole-processor units.
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn cores_since(&self, previous: (u64, u64, bool), booted: u16) -> f64 {
        let busy = self.ticks.0.saturating_sub(previous.0) as f64;
        let elapsed = self.ticks.1.saturating_sub(previous.1).max(1) as f64;
        if self.ticks.2 {
            // Host: process CPU ticks over wall-clock hundredths of a second.
            busy / elapsed
        } else {
            // Guest: busy share of every processor it booted.
            busy / elapsed * f64::from(booted.max(1))
        }
    }
}

/// Reads `instance`'s usage, from its holding process `host_pid` when this host can see it.
pub(super) fn measure<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &InstanceId,
    host_pid: u32,
) -> Option<Reading> {
    if host_pid != 0
        && let Some(reading) = from_host(host_pid)
    {
        return Some(reading);
    }
    let stat = read(facade, instance, "/proc/stat")?;
    let meminfo = read(facade, instance, "/proc/meminfo")?;
    let (busy, total) = cpu_ticks(&stat)?;
    let total_kb = meminfo_kb(&meminfo, "MemTotal:")?;
    let available_kb = meminfo_kb(&meminfo, "MemAvailable:")?;
    Some(Reading {
        ticks: (busy, total, false),
        memory_used_mb: total_kb.saturating_sub(available_kb) / 1024,
        memory_total_mb: total_kb / 1024,
    })
}

fn from_host(pid: u32) -> Option<Reading> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may hold spaces; the fields after its closing parenthesis are fixed.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let busy = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let rss_kb = meminfo_kb(&status, "VmRSS:")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let hundredths = u64::try_from(now / 10).ok()?;
    Some(Reading {
        ticks: (busy, hundredths, true),
        memory_used_mb: (rss_kb / 1024).saturating_sub(MONITOR_OVERHEAD_MB),
        memory_total_mb: 0,
    })
}

fn read<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &InstanceId,
    path: &str,
) -> Option<String> {
    let operation = OperationId::new(uuid::Uuid::new_v4().simple().to_string()).ok()?;
    let request = FileMachineRequest::new(
        operation,
        instance.clone(),
        FileOperation::Read {
            path: path.as_bytes().to_vec(),
        },
    );
    match facade.file(request).ok()?.answer {
        FileAnswer::Read { bytes } => String::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// The busy and total ticks of the `cpu` line of `/proc/stat`.
fn cpu_ticks(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();
    // user nice system idle iowait irq softirq steal
    let idle = fields.get(3)? + fields.get(4).copied().unwrap_or(0);
    let total: u64 = fields.iter().take(8).sum();
    Some((total.saturating_sub(idle), total))
}

fn meminfo_kb(meminfo: &str, key: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with(key))?;
    line[key.len()..].split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guest_counters_read_as_busy_and_total_ticks() {
        let stat = "cpu  100 5 50 800 20 3 2 0 0 0\ncpu0 1 2 3 4\n";
        assert_eq!(cpu_ticks(stat), Some((160, 980)));
        assert_eq!(
            meminfo_kb(
                "MemTotal: 4096 kB\nMemAvailable: 1024 kB\n",
                "MemAvailable:"
            ),
            Some(1024)
        );
    }
}
