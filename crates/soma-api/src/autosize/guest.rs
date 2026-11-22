//! Reading a guest's own CPU and memory counters through the file operation.

use soma::{FileAnswer, FileMachineRequest, FileOperation, InstanceId, OperationId};

use crate::facade::SandboxFacade;

pub(super) fn read<F: SandboxFacade + ?Sized>(
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
pub(super) fn cpu_ticks(stat: &str) -> Option<(u64, u64)> {
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

pub(super) fn meminfo_kb(meminfo: &str, key: &str) -> Option<u64> {
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
