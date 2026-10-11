//! How big the guest's in-memory scratch filesystems are.
//!
//! `/tmp` (on a machine that keeps it in RAM) and `/dev/shm` are tmpfs mounts charged to guest
//! RAM as they fill, so the size is a ceiling and never a reservation. The sizes follow the
//! machine, so the 2 GiB shape the live browser runs in gets room for a browser and the 1 GiB
//! shape keeps exactly what every earlier Generation was captured with.

/// The size `/tmp` has always had as a tmpfs, and the least it is ever given.
const SCRATCH_TMPFS_FLOOR: u64 = 64 * MIB;
/// The most `/tmp` is given as a tmpfs. A tmpfs is charged to guest RAM as it fills, so the cap
/// is what keeps a runaway writer from taking the whole machine.
const SCRATCH_TMPFS_CEILING: u64 = 512 * MIB;
/// A machine no larger than this keeps the 64 MiB `/tmp` it has always had: it is the 1 GiB shape
/// every earlier Generation was captured at, and its RAM has no room to lend to a larger one.
const SCRATCH_LEGACY_RAM: u64 = GIB + GIB / 4;
/// Shared memory, `/dev/shm`: what a browser, a database, and anything built on POSIX shared
/// memory expects to find, and what the guest never mounted.
pub const SHM_DIRECTORY: &str = "/dev/shm";
/// The most `/dev/shm` is given. Like `/tmp` it is a tmpfs charged as it is used, so the size is
/// a ceiling and not a reservation.
const SHM_CEILING: u64 = GIB;
/// The least `/dev/shm` is given on any machine.
const SHM_FLOOR: u64 = 64 * MIB;
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
/// The mount options of a tmpfs `/tmp`, sized to the machine.
///
/// A machine at or below the 1 GiB shape keeps the 64 MiB it has always had, and one that is not
/// reporting its memory is treated the same way. A larger one gets a quarter of its RAM, up to
/// 512 MiB, because a browser and a package install both write more than 64 MiB into `/tmp`.
#[must_use]
pub fn scratch_tmpfs_options(total_bytes: Option<u64>) -> String {
    let size = match total_bytes {
        Some(total) if total > SCRATCH_LEGACY_RAM => {
            (total / 4).clamp(SCRATCH_TMPFS_FLOOR, SCRATCH_TMPFS_CEILING)
        }
        _ => SCRATCH_TMPFS_FLOOR,
    };
    format!("mode=1777,size={}m", size / MIB)
}

/// The mount options of `/dev/shm`: half the machine's RAM, between 64 MiB and 1 GiB.
#[must_use]
pub fn shm_options(total_bytes: Option<u64>) -> String {
    let size = total_bytes.map_or(SHM_FLOOR, |total| (total / 2).clamp(SHM_FLOOR, SHM_CEILING));
    format!("mode=1777,size={}m", size / MIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tmpfs_scratch_directory_grows_with_the_machine_but_the_old_shape_keeps_its_size() {
        let mib = |count: u64| Some(count * 1024 * 1024);
        // The 1 GiB shape (the kernel reports a little less than 1024 MiB) and anything not
        // reporting memory keep exactly what they had.
        assert_eq!(scratch_tmpfs_options(mib(1000)), "mode=1777,size=64m");
        assert_eq!(scratch_tmpfs_options(mib(1024)), "mode=1777,size=64m");
        assert_eq!(scratch_tmpfs_options(None), "mode=1777,size=64m");
        // The 2 GiB browser shape gets a quarter of its RAM.
        assert_eq!(scratch_tmpfs_options(mib(2000)), "mode=1777,size=500m");
        assert_eq!(scratch_tmpfs_options(mib(2048)), "mode=1777,size=512m");
        // And the ceiling holds however large the machine is.
        assert_eq!(scratch_tmpfs_options(mib(3072)), "mode=1777,size=512m");
    }

    #[test]
    fn shared_memory_is_half_the_ram_between_the_floor_and_one_gib() {
        let mib = |count: u64| Some(count * 1024 * 1024);
        assert_eq!(shm_options(mib(1000)), "mode=1777,size=500m");
        assert_eq!(shm_options(mib(2048)), "mode=1777,size=1024m");
        assert_eq!(shm_options(mib(16 * 1024)), "mode=1777,size=1024m");
        assert_eq!(shm_options(mib(64)), "mode=1777,size=64m");
        assert_eq!(shm_options(None), "mode=1777,size=64m");
    }
}
