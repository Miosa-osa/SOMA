//! Places the validated PVH kernel, optional initramfs, and boot pages into guest RAM.
//!
//! The kernel's `PT_LOAD` segments land at their declared physical addresses. The initramfs is
//! placed top-down, page-aligned, directly below the end of guest RAM and must not touch any
//! kernel segment. Every placement is checked against the layout before a byte is written.

use std::{fs::File, io::Read as _};

use super::{
    boot_info,
    elf::PvhKernel,
    error::{MachineError, Phase},
    layout::{
        CMDLINE_ADDRESS, KERNEL_START, MEMMAP_ADDRESS, MODULE_ADDRESS, PAGE_SIZE,
        START_INFO_ADDRESS,
    },
    memory::GuestRam,
};

/// Largest kernel image the loader reads; the pinned kernel is a few tens of MiB.
pub(crate) const KERNEL_IMAGE_LIMIT: u64 = 256 * 1024 * 1024;
/// Largest initramfs the loader accepts; the acceptance fixture is a few KiB.
pub(crate) const INITRAMFS_LIMIT: u64 = 64 * 1024 * 1024;

/// Where the loader put things, retained as evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoadedKernel {
    pub(crate) entry: u64,
    pub(crate) kernel_end: u64,
    pub(crate) initramfs: Option<(u64, u64)>,
    pub(crate) cmdline: String,
}

/// Reads one artifact completely, rejecting an empty file or one above `limit` bytes.
pub(crate) fn read_bounded(file: File, limit: u64) -> Result<Vec<u8>, MachineError> {
    let length = file
        .metadata()
        .map_err(|error| MachineError::io(Phase::ReadKernel, &error))?
        .len();
    if length == 0 || length > limit {
        return Err(MachineError::invalid(
            Phase::ReadKernel,
            "artifact is empty or exceeds its size bound",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| MachineError::io(Phase::ReadKernel, &error))?;
    Ok(bytes)
}

/// Loads `image` and the optional `initramfs` into `ram` and writes the PVH boot pages with
/// the already composed `cmdline`.
pub(crate) fn load_kernel(
    ram: &mut GuestRam,
    image: &[u8],
    initramfs: Option<&[u8]>,
    cmdline: &str,
) -> Result<LoadedKernel, MachineError> {
    let kernel = PvhKernel::parse(image)?;
    let mut kernel_end = KERNEL_START;
    for segment in kernel.segments() {
        if !ram
            .layout()
            .contains(segment.guest_address, segment.memory_size)
        {
            return Err(MachineError::invalid(
                Phase::LoadGuest,
                "kernel segment lies outside guest RAM",
            ));
        }
        let end = segment.file_offset.saturating_add(segment.file_size);
        let bytes = image
            .get(segment.file_offset..end)
            .ok_or_else(|| MachineError::invalid(Phase::LoadGuest, "kernel segment truncated"))?;
        ram.write(segment.guest_address, bytes)?;
        let file_size = u64::try_from(segment.file_size)
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "segment size overflow"))?;
        let bss_start = segment
            .guest_address
            .checked_add(file_size)
            .ok_or_else(|| MachineError::invalid(Phase::LoadGuest, "segment end overflow"))?;
        ram.zero(bss_start, segment.memory_size - file_size)?;
        kernel_end = kernel_end.max(segment.guest_end());
    }
    let placed = match initramfs {
        Some(bytes) => Some(place_initramfs(ram, bytes, kernel_end)?),
        None => None,
    };
    write_boot_pages(ram, placed, cmdline)?;
    Ok(LoadedKernel {
        entry: u64::from(kernel.entry()),
        kernel_end,
        initramfs: placed,
        cmdline: cmdline.to_owned(),
    })
}

/// Places the initramfs page-aligned below the end of RAM and above the kernel.
fn place_initramfs(
    ram: &mut GuestRam,
    bytes: &[u8],
    kernel_end: u64,
) -> Result<(u64, u64), MachineError> {
    let size = u64::try_from(bytes.len())
        .map_err(|_| MachineError::invalid(Phase::LoadGuest, "initramfs size overflow"))?;
    if size == 0 || size > INITRAMFS_LIMIT {
        return Err(MachineError::invalid(
            Phase::LoadGuest,
            "initramfs must be between 1 byte and 64 MiB",
        ));
    }
    let start = ram
        .layout()
        .ram_bytes()
        .checked_sub(size)
        .map(|start| start & !(PAGE_SIZE - 1))
        .filter(|start| *start >= kernel_end)
        .ok_or_else(|| {
            MachineError::invalid(Phase::LoadGuest, "initramfs does not fit above the kernel")
        })?;
    ram.write(start, bytes)?;
    Ok((start, size))
}

fn write_boot_pages(
    ram: &mut GuestRam,
    initramfs: Option<(u64, u64)>,
    cmdline: &str,
) -> Result<(), MachineError> {
    let memmap = boot_info::memmap(ram.layout())?;
    let entries = u32::try_from(memmap.len() / boot_info::MEMMAP_ENTRY_BYTES)
        .map_err(|_| MachineError::invalid(Phase::LoadGuest, "memmap overflow"))?;
    let modules = u32::from(initramfs.is_some());
    ram.write(START_INFO_ADDRESS, &boot_info::start_info(entries, modules))?;
    ram.write(MEMMAP_ADDRESS, &memmap)?;
    if let Some((address, size)) = initramfs {
        ram.write(MODULE_ADDRESS, &boot_info::module_entry(address, size))?;
    } else {
        ram.zero(MODULE_ADDRESS, PAGE_SIZE)?;
    }
    ram.write(CMDLINE_ADDRESS, &boot_info::cmdline(cmdline)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmdline::{self, BootNonce};
    use crate::x86_64::{
        elf::synthetic::{PF_R, Segment, SyntheticElf},
        error::MachineErrorKind,
        layout::{GuestLayout, MIN_RAM_BYTES},
    };

    fn ram() -> GuestRam {
        GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap()
    }

    fn line(initramfs: bool, nonce: Option<&BootNonce>) -> String {
        cmdline::compose(initramfs, nonce)
    }

    #[test]
    fn loads_kernel_and_places_initramfs_top_down() {
        let entry = u32::try_from(KERNEL_START).unwrap() + 8;
        let image = SyntheticElf::kernel(entry).build();
        let initramfs = vec![0xaa_u8; 5000];
        let nonce = BootNonce::new([1; 8]);
        let loaded = load_kernel(
            &mut ram(),
            &image,
            Some(&initramfs),
            &line(true, Some(&nonce)),
        )
        .unwrap();
        assert_eq!(loaded.entry, u64::from(entry));
        assert_eq!(loaded.kernel_end, KERNEL_START + 64 + 4096);
        let expected_start = (MIN_RAM_BYTES - 5000) & !(PAGE_SIZE - 1);
        assert_eq!(loaded.initramfs, Some((expected_start, 5000)));
        assert!(
            loaded
                .cmdline
                .ends_with("rdinit=/init soma.nonce=0101010101010101")
        );
    }

    #[test]
    fn loads_without_initramfs_or_nonce() {
        let entry = u32::try_from(KERNEL_START).unwrap();
        let image = SyntheticElf::kernel(entry).build();
        let loaded = load_kernel(&mut ram(), &image, None, &line(false, None)).unwrap();
        assert_eq!(loaded.initramfs, None);
        assert_eq!(loaded.cmdline, boot_info::DIAGNOSTIC_CMDLINE);
    }

    #[test]
    fn rejects_segments_outside_ram_and_oversized_initramfs() {
        let entry = u32::try_from(KERNEL_START).unwrap();
        let mut elf = SyntheticElf::kernel(entry);
        elf.segments.push(Segment {
            address: MIN_RAM_BYTES - 8,
            data: vec![0; 16],
            extra_memory: 0,
            flags: PF_R,
        });
        let error = load_kernel(&mut ram(), &elf.build(), None, &line(false, None)).unwrap_err();
        assert_eq!(error.phase(), Phase::LoadGuest);
        assert!(matches!(error.kind(), MachineErrorKind::Invalid(_)));

        let image = SyntheticElf::kernel(entry).build();
        assert!(load_kernel(&mut ram(), &image, Some(&[]), &line(true, None)).is_err());
        let huge = vec![0_u8; usize::try_from(MIN_RAM_BYTES).unwrap()];
        assert!(
            load_kernel(
                &mut ram(),
                &image,
                Some(&huge[..huge.len() - 4096]),
                &line(true, None)
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_an_initramfs_that_would_cover_the_kernel() {
        let entry = u32::try_from(KERNEL_START).unwrap();
        let mut elf = SyntheticElf::kernel(entry);
        elf.segments[0].extra_memory = MIN_RAM_BYTES - KERNEL_START - 64 - 8192;
        let initramfs = vec![1_u8; 12288];
        let error = load_kernel(
            &mut ram(),
            &elf.build(),
            Some(&initramfs),
            &line(true, None),
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not fit"));
    }

    #[test]
    fn elf_rejections_surface_as_typed_load_errors() {
        let error = load_kernel(&mut ram(), b"not an elf", None, &line(false, None)).unwrap_err();
        assert_eq!(error.phase(), Phase::LoadGuest);
        assert!(matches!(error.kind(), MachineErrorKind::Elf(_)));
    }
}
