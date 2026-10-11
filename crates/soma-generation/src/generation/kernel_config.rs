use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

/// Maximum accepted kernel configuration text size.
pub const MAX_CONFIG_BYTES: usize = 4 * 1024 * 1024;

/// Facilities that must be built in for Generation profile v1.
pub const REQUIRED_BUILTIN: &[&str] = &[
    "CONFIG_64BIT",
    "CONFIG_X86_64",
    "CONFIG_KVM_GUEST",
    "CONFIG_PVH",
    "CONFIG_VIRTIO",
    "CONFIG_VIRTIO_MMIO",
    "CONFIG_VIRTIO_BLK",
    "CONFIG_VIRTIO_NET",
    "CONFIG_VIRTIO_VSOCKETS",
    "CONFIG_VSOCKETS",
    "CONFIG_HW_RANDOM_VIRTIO",
    "CONFIG_EROFS_FS",
    "CONFIG_EXT4_FS",
    "CONFIG_OVERLAY_FS",
    "CONFIG_DEVTMPFS",
    "CONFIG_PROC_FS",
    "CONFIG_SYSFS",
    "CONFIG_TMPFS",
    "CONFIG_UNIX",
    "CONFIG_BLK_DEV_INITRD",
    "CONFIG_SERIAL_8250",
    "CONFIG_SERIAL_8250_CONSOLE",
];

/// Facilities that must be absent or disabled for Generation profile v1.
///
/// `CONFIG_MEMORY_HOTPLUG` is not in this list: it is admitted only together with the live-resize
/// facilities, see [`MEMORY_HOTPLUG`].
pub const FORBIDDEN: &[&str] = &[
    "CONFIG_MODULES",
    "CONFIG_PCI",
    "CONFIG_ACPI",
    "CONFIG_USB",
    "CONFIG_USB_SUPPORT",
    "CONFIG_SOUND",
    "CONFIG_DRM",
    "CONFIG_FB",
    "CONFIG_SCSI",
    "CONFIG_VIRTIO_BALLOON",
    "CONFIG_VIRTIO_PCI",
    "CONFIG_HOTPLUG_PCI",
];

/// Memory hotplug is excluded from profile v1 and admitted only as live memory resize.
///
/// Hotplug is a kernel facility with a large surface (a hotplug source, onlining, migration), so
/// it is not allowed on its own. A kernel that enables it must be the one live-resize kernel:
/// the only hotplug source is the virtio-mem device over the existing virtio-mmio transport
/// (no ACPI, no PCI, both stay forbidden), memory can be handed back (hot-remove), and pages can
/// be moved out of the blocks being returned (migration). Anything less is a hotplug kernel with
/// no way to use it or no way to give it back, and is refused.
///
/// A kernel without `CONFIG_MEMORY_HOTPLUG` is checked exactly as before, so every Generation
/// compiled before live resize still verifies unchanged.
pub const MEMORY_HOTPLUG: &str = "CONFIG_MEMORY_HOTPLUG";

/// Facilities that must accompany [`MEMORY_HOTPLUG`].
pub const LIVE_RESIZE_REQUIRED: &[&str] = &[
    "CONFIG_MEMORY_HOTREMOVE",
    "CONFIG_MIGRATION",
    "CONFIG_VIRTIO_MEM",
];

const PHYSICAL_START: &str = "CONFIG_PHYSICAL_START";
const REQUIRED_PHYSICAL_START: u64 = 0x0100_0000;

/// One kernel configuration that satisfied the profile v1 facility requirements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedKernelConfig {
    /// The digest of the exact configuration text.
    pub digest: Sha256Digest,
}

/// Verifies a Linux `.config` text against the profile v1 built-in and exclusion lists.
///
/// # Errors
///
/// Returns [`CompileErrorKind::LimitExceeded`] for oversized text,
/// [`CompileErrorKind::InvalidInput`] for malformed lines, and
/// [`CompileErrorKind::Unsupported`] when a required facility is missing, an excluded facility
/// is enabled, or `CONFIG_PHYSICAL_START` is not `0x1000000`.
pub fn verify_kernel_config(text: &[u8]) -> Result<VerifiedKernelConfig, CompileError> {
    if text.len() > MAX_CONFIG_BYTES {
        return Err(CompileError::new(
            CompilePhase::VerifyKernel,
            CompileErrorKind::LimitExceeded,
        ));
    }
    let text = std::str::from_utf8(text).map_err(|_| invalid())?;
    let mut enabled = Vec::new();
    let mut physical_start = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(invalid)?;
        if !key.starts_with("CONFIG_") || key.len() > 128 {
            return Err(invalid());
        }
        if key == PHYSICAL_START {
            physical_start = Some(parse_hex(value)?);
        }
        if value == "y" {
            enabled.push(key);
        } else if value == "m" {
            return Err(unsupported());
        }
    }
    for required in REQUIRED_BUILTIN {
        if !enabled.contains(required) {
            return Err(unsupported());
        }
    }
    for forbidden in FORBIDDEN {
        if enabled.contains(forbidden) {
            return Err(unsupported());
        }
    }
    if enabled.contains(&MEMORY_HOTPLUG) {
        for required in LIVE_RESIZE_REQUIRED {
            if !enabled.contains(required) {
                return Err(unsupported());
            }
        }
    }
    if physical_start != Some(REQUIRED_PHYSICAL_START) {
        return Err(unsupported());
    }
    Ok(VerifiedKernelConfig {
        digest: Sha256Digest::of(text.as_bytes()),
    })
}

fn parse_hex(value: &str) -> Result<u64, CompileError> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(invalid)?;
    if digits.is_empty() || digits.len() > 16 {
        return Err(invalid());
    }
    u64::from_str_radix(digits, 16).map_err(|_| invalid())
}

const fn invalid() -> CompileError {
    CompileError::new(CompilePhase::VerifyKernel, CompileErrorKind::InvalidInput)
}

const fn unsupported() -> CompileError {
    CompileError::new(CompilePhase::VerifyKernel, CompileErrorKind::Unsupported)
}
