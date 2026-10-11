//! Telling the kernel where memory that is plugged in later should go.
//!
//! A machine can be given more memory while it runs. The kernel decides what zone newly added
//! memory joins, and only memory in the movable zone can be taken back, so the agent asks for
//! that before the first block can arrive. The setting lives in the guest's memory, so it is
//! written on every boot and every restore; writing it twice changes nothing.
//!
//! A kernel built without memory hotplug has no such file, which is the ordinary case for an
//! older Generation, and is not an error.

use std::fs;
use std::io;
use std::path::Path;

/// Where the kernel exposes the online policy for newly added memory.
pub const AUTO_ONLINE: &str = "/sys/devices/system/memory/auto_online_blocks";
const MOVABLE: &str = "online_movable";

/// What the write did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// New memory will come online in the movable zone.
    Set,
    /// This kernel has no memory hotplug, so there is nothing to set.
    Absent,
    /// The kernel refused the value.
    Refused,
}

/// Asks the kernel to put newly added memory in the movable zone.
#[must_use]
pub fn online_movable(path: &Path) -> Outcome {
    match fs::write(path, MOVABLE) {
        Ok(()) => Outcome::Set,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Outcome::Absent,
        Err(_) => Outcome::Refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_movable_policy_is_written_and_a_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("dir");
        let file = dir.path().join("auto_online_blocks");
        fs::write(&file, "offline").expect("seed");
        assert_eq!(online_movable(&file), Outcome::Set);
        assert_eq!(fs::read_to_string(&file).expect("read"), MOVABLE);
        assert_eq!(
            online_movable(&file),
            Outcome::Set,
            "writing twice changes nothing"
        );
        assert_eq!(
            online_movable(&dir.path().join("absent/auto_online_blocks")),
            Outcome::Absent
        );
        assert_eq!(online_movable(dir.path()), Outcome::Refused);
    }
}
