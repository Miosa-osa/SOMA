//! What one host has left for sandboxes: processors, memory, disk, and how many more fit.
//!
//! The numbers are read from the host when asked, not tracked, so they include everything else
//! the host runs. Guest memory is mapped from snapshot objects and faulted in on use, so the
//! available memory the kernel reports is what the running sandboxes actually hold, and "fit" is
//! stated against it with the reserve the host keeps for itself.

use std::fs;
use std::path::Path;

use serde::Serialize;

/// Memory the host keeps for itself and is never counted as room for a sandbox.
pub const HOST_RESERVE_MIB: u64 = 8 * 1024;

/// One host's capacity, as read now.
#[derive(Clone, Debug, Serialize)]
pub struct HostCapacity {
    pub cpus: u64,
    /// The one-minute load average, the closest a host has to "how busy are the processors".
    pub load_1m: Option<f64>,
    pub memory_total_mib: Option<u64>,
    pub memory_available_mib: Option<u64>,
    pub reserve_mib: u64,
    /// Bytes free on the filesystem of the state root, where heads and Suspensions live.
    pub state_disk: Option<Disk>,
    /// Bytes free on the filesystem of the Generation store.
    pub generation_disk: Option<Disk>,
}

/// One filesystem's size and what is still free on it.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Disk {
    pub total_mib: u64,
    pub available_mib: u64,
}

impl HostCapacity {
    /// How many more sandboxes of `memory_mib` fit in the memory left above the reserve, or
    /// `None` when the host's memory could not be read.
    #[must_use]
    pub fn fits(&self, memory_mib: u64) -> Option<u64> {
        let available = self.memory_available_mib?;
        Some(available.saturating_sub(self.reserve_mib) / memory_mib.max(1))
    }
}

/// Reads the host now: `state_root` is the runtime's, the Generation store is the configured one.
#[must_use]
pub fn read(state_root: &Path) -> HostCapacity {
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let generation_store = std::env::var_os("SOMA_GENERATION_STORE");
    HostCapacity {
        cpus: std::thread::available_parallelism().map_or(0, |count| count.get() as u64),
        load_1m: fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|text| text.split_whitespace().next()?.parse().ok()),
        memory_total_mib: meminfo_mib(&meminfo, "MemTotal:"),
        memory_available_mib: meminfo_mib(&meminfo, "MemAvailable:"),
        reserve_mib: HOST_RESERVE_MIB,
        state_disk: disk(state_root),
        generation_disk: generation_store.as_deref().map(Path::new).and_then(disk),
    }
}

fn meminfo_mib(meminfo: &str, key: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with(key))?;
    let kib: u64 = line[key.len()..].split_whitespace().next()?.parse().ok()?;
    Some(kib / 1024)
}

#[cfg(target_os = "linux")]
fn disk(path: &Path) -> Option<Disk> {
    soma_storage::space::of(path).ok().map(|space| Disk {
        total_mib: space.total_bytes / (1024 * 1024),
        available_mib: space.available_bytes / (1024 * 1024),
    })
}

#[cfg(not(target_os = "linux"))]
const fn disk(_path: &Path) -> Option<Disk> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_lines_read_in_mebibytes() {
        let text = "MemTotal:       394549324 kB\nMemFree:  1 kB\nMemAvailable:   385875412 kB\n";
        assert_eq!(meminfo_mib(text, "MemTotal:"), Some(385_302));
        assert_eq!(meminfo_mib(text, "MemAvailable:"), Some(376_831));
        assert_eq!(meminfo_mib(text, "SwapTotal:"), None);
    }

    #[test]
    fn a_fit_leaves_the_reserve() {
        let host = HostCapacity {
            cpus: 4,
            load_1m: None,
            memory_total_mib: Some(64 * 1024),
            memory_available_mib: Some(HOST_RESERVE_MIB + 10 * 1024 + 5),
            reserve_mib: HOST_RESERVE_MIB,
            state_disk: None,
            generation_disk: None,
        };
        assert_eq!(host.fits(1024), Some(10));
        assert_eq!(host.fits(0), Some(10 * 1024 + 5));
    }
}
