//! Run identity captured before the first sample.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::profile::StorageProfile;

/// The mount that receives heads, read from `/proc/self/mountinfo`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountInfo {
    /// Kernel mount id.
    pub mount_id: u64,
    /// Mount point.
    pub mount_point: String,
    /// Filesystem type.
    pub fs_type: String,
    /// Mount source, for a loop device the `/dev/loopN` node.
    pub source: String,
    /// Per-mount options.
    pub mount_options: String,
    /// Superblock options.
    pub super_options: String,
}

/// Everything the evidence document needs to attribute the samples.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    /// Kernel release.
    pub kernel: String,
    /// CPU model name.
    pub cpu_model: String,
    /// Online logical CPUs.
    pub cpus: usize,
    /// Head filesystem mount.
    pub mount: MountInfo,
    /// Device number of the head directory.
    pub device: u64,
    /// Free bytes at the start of the run.
    pub free_bytes: u64,
    /// Loop device from `SOMA_XFS_LOOP_DEVICE`, empty when not loop-backed.
    pub loop_device: String,
    /// Backing file of the loop device from `SOMA_XFS_BACKING_FILE`.
    pub backing_file: String,
    /// Git revision from `SOMA_GIT_REV`.
    pub git_rev: String,
    /// Seconds since the Unix epoch when the run started.
    pub started_unix_s: u64,
    /// Percentile method used by every summary.
    pub percentile_method: String,
    /// Samples requested per cell.
    pub samples_per_cell: usize,
}

impl RunIdentity {
    /// Gathers the identity for a run whose head directory is `dir` with probed `profile`.
    ///
    /// # Errors
    ///
    /// Propagates failures to read the kernel or mount tables.
    pub fn gather(
        dir: &Path,
        profile: &StorageProfile,
        samples_per_cell: usize,
    ) -> io::Result<Self> {
        let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")?
            .trim()
            .to_owned();
        let cpuinfo = fs::read_to_string("/proc/cpuinfo")?;
        let cpu_model = cpuinfo
            .lines()
            .find_map(|line| line.strip_prefix("model name"))
            .and_then(|rest| rest.split_once(':'))
            .map(|(_, model)| model.trim().to_owned())
            .unwrap_or_default();
        let cpus = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);
        let mount = mount_info(
            &fs::read_to_string("/proc/self/mountinfo")?,
            profile.mount_id(),
            dir,
        );
        let started_unix_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Ok(Self {
            kernel,
            cpu_model,
            cpus,
            mount,
            device: profile.device(),
            free_bytes: profile.free_bytes(),
            loop_device: std::env::var("SOMA_XFS_LOOP_DEVICE").unwrap_or_default(),
            backing_file: std::env::var("SOMA_XFS_BACKING_FILE").unwrap_or_default(),
            git_rev: std::env::var("SOMA_GIT_REV").unwrap_or_default(),
            started_unix_s,
            percentile_method: "nearest-rank".to_owned(),
            samples_per_cell,
        })
    }
}

/// Finds the `mountinfo` line with `mount_id`, falling back to the longest mount point that
/// prefixes `dir`.
#[must_use]
pub fn mount_info(mountinfo: &str, mount_id: u64, dir: &Path) -> MountInfo {
    let parsed: Vec<MountInfo> = mountinfo.lines().filter_map(parse_line).collect();
    if let Some(found) = parsed.iter().find(|m| m.mount_id == mount_id) {
        return found.clone();
    }
    let dir_text = dir.to_string_lossy();
    parsed
        .into_iter()
        .filter(|m| dir_text.starts_with(&m.mount_point))
        .max_by_key(|m| m.mount_point.len())
        .unwrap_or_default()
}

fn parse_line(line: &str) -> Option<MountInfo> {
    let (before, after) = line.split_once(" - ")?;
    let before: Vec<&str> = before.split(' ').collect();
    let after: Vec<&str> = after.split(' ').collect();
    if before.len() < 6 || after.len() < 3 {
        return None;
    }
    Some(MountInfo {
        mount_id: before[0].parse().ok()?,
        mount_point: unescape(before[4]),
        mount_options: before[5].to_owned(),
        fs_type: after[0].to_owned(),
        source: after[1].to_owned(),
        super_options: after[2].to_owned(),
    })
}

fn unescape(text: &str) -> String {
    text.replace("\\040", " ").replace("\\011", "\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
1157 30 7:28 / /mnt/soma/reflink rw,noatime shared:600 - xfs /dev/loop28 rw,attr2,inode64,logbufs=8,logbsize=32k,noquota
30 1 0:25 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw,errors=remount-ro
";

    #[test]
    fn finds_the_mount_by_id_or_by_longest_prefix() {
        let by_id = mount_info(SAMPLE, 1157, Path::new("/elsewhere"));
        assert_eq!(by_id.fs_type, "xfs");
        assert_eq!(by_id.source, "/dev/loop28");
        assert_eq!(by_id.mount_options, "rw,noatime");
        assert!(by_id.super_options.contains("logbsize=32k"));

        let by_prefix = mount_info(SAMPLE, 99, Path::new("/mnt/soma/reflink/heads"));
        assert_eq!(by_prefix.mount_id, 1157);
        let root = mount_info(SAMPLE, 99, Path::new("/tmp"));
        assert_eq!(root.mount_id, 30);
        assert_eq!(
            mount_info("garbage", 1, Path::new("/")),
            MountInfo::default()
        );
    }
}
