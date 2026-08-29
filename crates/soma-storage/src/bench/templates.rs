//! Benchmark templates: one sterile, one fully preallocated, and one fragmented ext4 template
//! per size.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::BenchError;
use super::cell::{Allocation, TemplateSize};
use super::record::TemplateRecord;
use super::stats::nanos;
use crate::fiemap;
use crate::profile::{
    BlockSize, ClassName, Ext4FeatureSet, InodePolicy, LogicalBytes, MountOption, MountOptions,
    OverlayRecipe, UuidPolicy,
};
use crate::template;

/// One template ready for cloning.
#[derive(Debug)]
pub struct BenchTemplate {
    /// Size dimension.
    pub size: TemplateSize,
    /// Allocation dimension.
    pub allocation: Allocation,
    /// Location inside the template store, used only by the `cp` comparison.
    pub path: PathBuf,
    /// Read-only descriptor used by `FICLONE`.
    pub file: File,
    /// Identity record written to the JSONL output.
    pub record: TemplateRecord,
}

/// The overlay recipe of one benchmark template.
///
/// # Panics
///
/// Does not panic in practice: the fixed name, size, and inode ratio are accepted by every
/// validator, which the unit tests prove.
#[must_use]
pub fn recipe(size: TemplateSize, allocation: Allocation) -> OverlayRecipe {
    let name = format!("bench-{}-{}", size.label(), allocation.label());
    OverlayRecipe {
        name: ClassName::new(name).expect("benchmark class names are valid"),
        version: 1,
        logical_bytes: LogicalBytes::new(size.bytes(), BlockSize::B4096)
            .expect("benchmark sizes are aligned and in range"),
        block_size: BlockSize::B4096,
        uuid_policy: UuidPolicy::Derived,
        features: Ext4FeatureSet::V1,
        inode_policy: InodePolicy::bytes_per_inode(16384).expect("16 KiB per inode is accepted"),
        mount_options: MountOptions::new(&[
            MountOption::NoAtime,
            MountOption::NoDev,
            MountOption::NoSuid,
            MountOption::DataOrdered,
            MountOption::ErrorsRemountRo,
        ]),
    }
}

/// Creates every template for `sizes` inside `store`.
///
/// # Errors
///
/// Returns the first template that could not be created, preallocated, or summarized.
pub fn prepare(store: &Path, sizes: &[TemplateSize]) -> Result<Vec<BenchTemplate>, BenchError> {
    let mut templates = Vec::new();
    for &size in sizes {
        for allocation in [
            Allocation::Sterile,
            Allocation::Preallocated,
            Allocation::Fragmented,
        ] {
            templates.push(prepare_one(store, size, allocation)?);
        }
    }
    Ok(templates)
}

fn prepare_one(
    store: &Path,
    size: TemplateSize,
    allocation: Allocation,
) -> Result<BenchTemplate, BenchError> {
    let recipe = recipe(size, allocation);
    let started = Instant::now();
    let sterile = template::create_template(store, &recipe).map_err(BenchError::Template)?;
    match allocation {
        Allocation::Sterile => {}
        Allocation::Preallocated => {
            preallocate(sterile.path(), 0, size.bytes(), size.bytes())
                .map_err(|e| BenchError::Io("fallocate", e))?;
        }
        Allocation::Fragmented => {
            preallocate(
                sterile.path(),
                FRAGMENT_STRIDE,
                FRAGMENT_BYTES,
                size.bytes(),
            )
            .map_err(|e| BenchError::Io("fragmenting fallocate", e))?;
        }
    }
    let creation_ns = nanos(started.elapsed());
    let digest =
        template::digest_file(sterile.path()).map_err(|e| BenchError::Io("template digest", e))?;
    let file = File::open(sterile.path()).map_err(|e| BenchError::Io("template open", e))?;
    let extents = fiemap::summarize(std::os::fd::AsFd::as_fd(&file))
        .map_err(|e| BenchError::Io("template fiemap", e))?;
    let record = TemplateRecord {
        size,
        allocation,
        class: recipe.name.as_str().to_owned(),
        digest: digest.to_string(),
        bytes: size.bytes(),
        extents,
        creation_ns,
    };
    Ok(BenchTemplate {
        size,
        allocation,
        path: sterile.path().to_path_buf(),
        file,
        record,
    })
}

/// Logical distance between fragments.
const FRAGMENT_STRIDE: u64 = 128 * 1024;
/// Bytes allocated per fragment.
const FRAGMENT_BYTES: u64 = 4096;

/// Allocates `length` bytes at every `stride` offset below `total`; `stride == 0` means one
/// allocation of the complete range.
///
/// Unwritten extents read as zero, so the template bytes and digest do not change.
fn preallocate(path: &Path, stride: u64, length: u64, total: u64) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    let mut offset = 0u64;
    while offset < total {
        let len = length.min(total - offset);
        let c_offset = libc::off_t::try_from(offset).map_err(|_| io::Error::other("offset"))?;
        let c_len = libc::off_t::try_from(len).map_err(|_| io::Error::other("length"))?;
        // SAFETY: `file` is a live writable descriptor and mode 0 with a checked offset and
        // length asks the kernel only to allocate space inside the existing file.
        if unsafe { libc::fallocate(file.as_raw_fd(), 0, c_offset, c_len) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if stride == 0 {
            break;
        }
        offset += stride;
    }
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_name_size_and_allocation() {
        let sterile = recipe(TemplateSize::Gib1, Allocation::Sterile);
        assert_eq!(sterile.name.as_str(), "bench-1g-sterile");
        assert_eq!(sterile.logical_bytes.get(), 1024 * 1024 * 1024);
        let full = recipe(TemplateSize::Mib100, Allocation::Preallocated);
        assert_eq!(full.name.as_str(), "bench-100m-prealloc");
        assert_eq!(
            recipe(TemplateSize::Gib4, Allocation::Fragmented)
                .name
                .as_str(),
            "bench-4g-frag"
        );
        assert_eq!(
            full.mount_options.render(),
            "noatime,nodev,nosuid,data=ordered,errors=remount-ro"
        );
    }
}
