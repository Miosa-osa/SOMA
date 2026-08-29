//! Device status register bits and the driver-visible lifecycle order.

use std::fmt;

pub const STATUS_ACKNOWLEDGE: u8 = 1;
pub const STATUS_DRIVER: u8 = 2;
pub const STATUS_DRIVER_OK: u8 = 4;
pub const STATUS_FEATURES_OK: u8 = 8;
pub const STATUS_DEVICE_NEEDS_RESET: u8 = 64;
pub const STATUS_FAILED: u8 = 128;

const KNOWN_BITS: u8 = STATUS_ACKNOWLEDGE
    | STATUS_DRIVER
    | STATUS_DRIVER_OK
    | STATUS_FEATURES_OK
    | STATUS_DEVICE_NEEDS_RESET
    | STATUS_FAILED;
const DRIVER_SETTABLE: u8 =
    STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK | STATUS_FEATURES_OK | STATUS_FAILED;

/// Why a status write was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusViolation {
    /// The value has bits outside the specification.
    UnknownBits { value: u64 },
    /// The write would clear a bit without a full reset.
    ClearedBits { current: u8, value: u8 },
    /// The write sets more than one new bit at once.
    MultipleBits { value: u8 },
    /// The driver tried to set `DEVICE_NEEDS_RESET`.
    DriverSetNeedsReset,
    /// The new bit's prerequisite bit is not set.
    OutOfOrder { bit: u8 },
}

impl fmt::Display for StatusViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "status violation: {self:?}")
    }
}

impl std::error::Error for StatusViolation {}

/// The device status register.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeviceStatus(u8);

/// What a validated status write asks the transport to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusWrite {
    /// The driver wrote zero: perform a full reset.
    Reset,
    /// No bit changed.
    Unchanged,
    /// Exactly this bit was newly set.
    SetBit(u8),
}

impl DeviceStatus {
    /// Raw bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Builds from captured bits, rejecting unknown or out-of-order bits.
    ///
    /// # Errors
    /// Returns the typed rejection.
    pub const fn from_bits(bits: u8) -> Result<Self, StatusViolation> {
        if bits & !KNOWN_BITS != 0 {
            return Err(StatusViolation::UnknownBits { value: bits as u64 });
        }
        let status = Self(bits);
        let chain = [
            STATUS_DRIVER_OK,
            STATUS_FEATURES_OK,
            STATUS_DRIVER,
            STATUS_ACKNOWLEDGE,
        ];
        let mut index = 0;
        while index < chain.len() {
            let bit = chain[index];
            if status.has(bit) && !status.has(prerequisite(bit)) {
                return Err(StatusViolation::OutOfOrder { bit });
            }
            index += 1;
        }
        Ok(status)
    }

    /// Whether `bit` is set.
    #[must_use]
    pub const fn has(self, bit: u8) -> bool {
        self.0 & bit == bit
    }

    /// Whether the driver has completed initialization.
    #[must_use]
    pub const fn driver_ok(self) -> bool {
        self.has(STATUS_DRIVER_OK)
    }

    /// Whether feature negotiation is locked.
    #[must_use]
    pub const fn features_ok(self) -> bool {
        self.has(STATUS_FEATURES_OK)
    }

    /// Whether the device has requested a reset or the driver has given up.
    #[must_use]
    pub const fn is_failed(self) -> bool {
        self.has(STATUS_DEVICE_NEEDS_RESET) || self.has(STATUS_FAILED)
    }

    /// Sets one bit unconditionally; used by the transport for device-side bits.
    #[must_use]
    pub const fn with(self, bit: u8) -> Self {
        Self(self.0 | bit)
    }

    /// Validates a driver status write against the lifecycle order.
    ///
    /// # Errors
    /// Returns the typed rejection; the status is unchanged.
    pub fn classify_write(self, value: u64) -> Result<StatusWrite, StatusViolation> {
        if value == 0 {
            return Ok(StatusWrite::Reset);
        }
        let Some(value) = u8::try_from(value)
            .ok()
            .filter(|bits| bits & !KNOWN_BITS == 0)
        else {
            return Err(StatusViolation::UnknownBits { value });
        };
        if self.0 & !value != 0 {
            return Err(StatusViolation::ClearedBits {
                current: self.0,
                value,
            });
        }
        let new_bits = value & !self.0;
        if new_bits == 0 {
            return Ok(StatusWrite::Unchanged);
        }
        if new_bits & STATUS_DEVICE_NEEDS_RESET != 0 {
            return Err(StatusViolation::DriverSetNeedsReset);
        }
        if new_bits.count_ones() != 1 || new_bits & !DRIVER_SETTABLE != 0 {
            return Err(StatusViolation::MultipleBits { value });
        }
        if new_bits != STATUS_FAILED && !self.has(prerequisite(new_bits)) {
            return Err(StatusViolation::OutOfOrder { bit: new_bits });
        }
        Ok(StatusWrite::SetBit(new_bits))
    }
}

/// The bit that must already be set before `bit` may be set; zero for none.
const fn prerequisite(bit: u8) -> u8 {
    match bit {
        STATUS_DRIVER => STATUS_ACKNOWLEDGE,
        STATUS_FEATURES_OK => STATUS_DRIVER,
        STATUS_DRIVER_OK => STATUS_FEATURES_OK,
        _ => 0,
    }
}
