//! Asking a live guest to take or give back memory, through the device that backs it.
//!
//! The device and the guest driver do the work; this is the seam the session talks to. The host
//! states how much extra memory it wants plugged on top of what the machine booted with, and the
//! device reports how much is actually plugged. The two differ while the guest is still working
//! and may stay different, so a caller reads the plugged number back and reports that, never the
//! number it asked for.

/// What the device holds at one moment, in bytes beyond the memory the machine booted with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryLevel {
    /// How much the host has asked to be plugged.
    pub requested_bytes: u64,
    /// How much the guest has actually plugged.
    pub plugged_bytes: u64,
    /// The most that can ever be plugged: the size of the device's region.
    pub region_bytes: u64,
}

/// Why the memory device did not take a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    /// The machine was built without a memory device.
    NoDevice,
    /// The size is not a whole number of blocks, or is beyond the region.
    BadSize,
    /// A checkpoint or fork is in flight, and plugged memory is not in a snapshot.
    Busy,
}

/// The memory device of one machine, as the session needs it.
///
/// The three methods are the ones the machine exposes; `region_bytes` tells a caller the ceiling
/// without asking it to know the device's geometry.
pub trait MemoryControl {
    /// States how much memory should be plugged on top of the boot memory.
    ///
    /// # Errors
    ///
    /// Returns the [`MemoryError`] naming why the device refused the size.
    fn set_requested_memory(&self, bytes: u64) -> Result<(), MemoryError>;
    /// How much the guest has plugged.
    fn plugged_memory(&self) -> u64;
    /// How much the host last asked for.
    fn requested_memory(&self) -> u64;
    /// The size of the device's region; zero when there is no device.
    fn region_bytes(&self) -> u64;
}

/// The machine of a Generation built without a memory device.
#[derive(Clone, Copy, Debug)]
pub struct NoMemoryDevice;

impl MemoryControl for NoMemoryDevice {
    fn set_requested_memory(&self, _bytes: u64) -> Result<(), MemoryError> {
        Err(MemoryError::NoDevice)
    }

    fn plugged_memory(&self) -> u64 {
        0
    }

    fn requested_memory(&self) -> u64 {
        0
    }

    fn region_bytes(&self) -> u64 {
        0
    }
}

/// The memory device of a running machine.
///
/// A machine whose Generation declared no region answers `NoDevice` and a region of zero bytes,
/// so a caller can tell "cannot" from "refused" without knowing the device's geometry.
pub struct MachineMemory<'a>(&'a soma_kvm::x86_64::SandboxMachine);

impl MemoryControl for MachineMemory<'_> {
    fn set_requested_memory(&self, bytes: u64) -> Result<(), MemoryError> {
        self.0
            .set_requested_memory(bytes)
            .map_err(|error| match error {
                soma_kvm::SizeError::NoDevice => MemoryError::NoDevice,
                soma_kvm::SizeError::Unaligned | soma_kvm::SizeError::BeyondRegion => {
                    MemoryError::BadSize
                }
            })
    }

    fn plugged_memory(&self) -> u64 {
        self.0.plugged_memory()
    }

    fn requested_memory(&self) -> u64 {
        self.0.requested_memory()
    }

    fn region_bytes(&self) -> u64 {
        self.0.hotplug_region().map_or(0, |region| region.size)
    }
}

/// The memory device of a machine.
#[must_use]
pub const fn device_of(machine: &soma_kvm::x86_64::SandboxMachine) -> MachineMemory<'_> {
    MachineMemory(machine)
}

/// Reads the level off any device.
pub fn level(device: &dyn MemoryControl) -> MemoryLevel {
    MemoryLevel {
        requested_bytes: device.requested_memory(),
        plugged_bytes: device.plugged_memory(),
        region_bytes: device.region_bytes(),
    }
}

/// Applies a request to a device, then reads the level back.
///
/// # Errors
///
/// Returns the [`MemoryError`] the device refused with.
pub fn apply(device: &dyn MemoryControl, bytes: u64) -> Result<MemoryLevel, MemoryError> {
    device.set_requested_memory(bytes)?;
    Ok(level(device))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    struct Fake {
        requested: Cell<u64>,
        region: u64,
    }

    impl MemoryControl for Fake {
        fn set_requested_memory(&self, bytes: u64) -> Result<(), MemoryError> {
            if bytes > self.region {
                return Err(MemoryError::BadSize);
            }
            self.requested.set(bytes);
            Ok(())
        }
        fn plugged_memory(&self) -> u64 {
            0
        }
        fn requested_memory(&self) -> u64 {
            self.requested.get()
        }
        fn region_bytes(&self) -> u64 {
            self.region
        }
    }

    #[test]
    fn a_machine_without_the_device_refuses_every_size() {
        assert_eq!(apply(&NoMemoryDevice, 1 << 21), Err(MemoryError::NoDevice));
        assert_eq!(level(&NoMemoryDevice), MemoryLevel::default());
    }

    #[test]
    fn the_level_reports_what_the_device_holds_not_what_was_asked() {
        let device = Fake {
            requested: Cell::new(0),
            region: 4 << 21,
        };
        let seen = apply(&device, 2 << 21).expect("accepted");
        assert_eq!(seen.requested_bytes, 2 << 21);
        assert_eq!(
            seen.plugged_bytes, 0,
            "the guest has not plugged anything yet"
        );
        assert_eq!(apply(&device, 5 << 21), Err(MemoryError::BadSize));
    }
}
