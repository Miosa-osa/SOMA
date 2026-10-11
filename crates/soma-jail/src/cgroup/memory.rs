//! Moving `memory.max` around a plug or an unplug of guest memory, in the only safe order.
//!
//! The limit has to be above what the guest will use before the guest is told to use it, and may
//! only come down once the guest has given the memory back. A limit written under the live usage
//! makes the kernel reclaim and then kill the machine, so the lowering side refuses it outright
//! instead of leaving that to the kernel.

use super::{CgroupError, CgroupLeaf, parse_max};

impl CgroupLeaf {
    /// Raises `memory.max` to `bytes`. Run this BEFORE plugging memory into the guest, so the
    /// plugged pages never meet a limit that is still the old one.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::InvalidResize`] if `bytes` is below the current limit, or the write
    /// and readback failures of the underlying write.
    pub fn raise_memory_max(&self, bytes: u64) -> Result<u64, CgroupError> {
        let current = parse_max("memory.max", &self.read("memory.max")?)?;
        if bytes < current {
            return Err(CgroupError::InvalidResize("raise would lower memory.max"));
        }
        self.write_memory_max(bytes)
    }

    /// Lowers `memory.max` to `bytes`. Run this only AFTER the guest has unplugged the memory.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::InvalidResize`] if `bytes` is above the current limit or below
    /// `memory.current`, or the write and readback failures of the underlying write.
    pub fn lower_memory_max(&self, bytes: u64) -> Result<u64, CgroupError> {
        let current = parse_max("memory.max", &self.read("memory.max")?)?;
        if bytes > current {
            return Err(CgroupError::InvalidResize("lower would raise memory.max"));
        }
        let used = parse_max("memory.current", &self.read("memory.current")?)?;
        if bytes < used {
            return Err(CgroupError::InvalidResize(
                "memory.max would sit below memory.current",
            ));
        }
        self.write_memory_max(bytes)
    }

    fn write_memory_max(&self, bytes: u64) -> Result<u64, CgroupError> {
        self.write("memory.max", &bytes.to_string())?;
        let found = parse_max("memory.max", &self.read("memory.max")?)?;
        if found == bytes {
            Ok(found)
        } else {
            Err(CgroupError::Readback {
                file: "memory.max",
                expected: bytes.to_string(),
                found: found.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn leaf(memory: &str, current: &str) -> (tempfile::TempDir, CgroupLeaf) {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir(root.path().join("soma-m")).expect("leaf");
        fs::write(root.path().join("soma-m/memory.max"), memory).expect("memory.max");
        fs::write(root.path().join("soma-m/memory.current"), current).expect("memory.current");
        let leaf = CgroupLeaf::open_existing(root.path(), "soma-m").expect("open");
        (root, leaf)
    }

    #[test]
    fn memory_is_raised_before_a_plug_and_lowered_only_after_an_unplug() {
        let (root, leaf) = leaf("2000", "1500");
        assert_eq!(leaf.raise_memory_max(4000), Ok(4000));
        assert_eq!(
            leaf.raise_memory_max(3000),
            Err(CgroupError::InvalidResize("raise would lower memory.max"))
        );
        assert_eq!(
            leaf.lower_memory_max(5000),
            Err(CgroupError::InvalidResize("lower would raise memory.max"))
        );
        assert_eq!(
            leaf.lower_memory_max(1000),
            Err(CgroupError::InvalidResize(
                "memory.max would sit below memory.current"
            )),
            "a limit under the live usage must be refused, not written"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("soma-m/memory.max")).expect("read"),
            "4000"
        );
        fs::write(root.path().join("soma-m/memory.current"), "800").expect("unplugged");
        assert_eq!(leaf.lower_memory_max(1000), Ok(1000));
    }
}
