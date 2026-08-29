pub(crate) const RAM_BASE: u64 = 0x8000_0000;
pub(crate) const RAM_SIZE: u64 = 128 * 1024 * 1024;
pub(crate) const KERNEL_BASE: u64 = RAM_BASE + 2 * 1024 * 1024;
pub(crate) const FDT_MAX_SIZE: u64 = 2 * 1024 * 1024;
pub(crate) const PAGE_SIZE: u64 = 4096;

pub(crate) const GIC_DIST_BASE: u64 = 0x0800_0000;
pub(crate) const GIC_DIST_SIZE: u64 = 0x0001_0000;
pub(crate) const GIC_REDIST_BASE: u64 = 0x080a_0000;
pub(crate) const GIC_REDIST_SIZE: u64 = 0x0002_0000;
pub(crate) const UART_BASE: u64 = 0x0900_0000;
pub(crate) const UART_SIZE: u64 = 0x1000;
pub(crate) const UART_SPI: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BootLayout {
    pub(crate) initrd_start: u64,
    pub(crate) initrd_end: u64,
    pub(crate) fdt_start: u64,
}

impl BootLayout {
    pub(crate) fn new(initrd_size: usize) -> Result<Self, &'static str> {
        let initrd_size = u64::try_from(initrd_size).map_err(|_| "initramfs size overflow")?;
        if initrd_size == 0 {
            return Err("initramfs is empty");
        }
        let ram_end = RAM_BASE.checked_add(RAM_SIZE).ok_or("RAM end overflow")?;
        let fdt_start = ram_end
            .checked_sub(FDT_MAX_SIZE)
            .ok_or("FDT does not fit in RAM")?;
        let unaligned_start = fdt_start
            .checked_sub(initrd_size)
            .ok_or("initramfs does not fit below the FDT")?;
        let initrd_start = unaligned_start & !(PAGE_SIZE - 1);
        let initrd_end = initrd_start
            .checked_add(initrd_size)
            .ok_or("initramfs end overflow")?;
        if initrd_start <= KERNEL_BASE || initrd_end > fdt_start {
            return Err("initramfs does not fit in guest RAM");
        }
        Ok(Self {
            initrd_start,
            initrd_end,
            fdt_start,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_initramfs_before_reserved_fdt_space() {
        let layout = BootLayout::new(8193).unwrap();
        assert_eq!(layout.initrd_start % PAGE_SIZE, 0);
        assert_eq!(layout.initrd_end - layout.initrd_start, 8193);
        assert!(layout.initrd_end <= layout.fdt_start);
        assert_eq!(layout.fdt_start, RAM_BASE + RAM_SIZE - FDT_MAX_SIZE);
    }

    #[test]
    fn rejects_empty_or_oversized_initramfs() {
        assert!(BootLayout::new(0).is_err());
        assert!(BootLayout::new(usize::MAX).is_err());
    }
}
