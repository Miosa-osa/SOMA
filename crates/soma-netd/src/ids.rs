//! Opaque validated identities that cross the broker seam.
//!
//! The broker is independent of the VMM crate, so it owns its own fixed-width identities.
//! Every identity rejects the all-zero value and exposes exact bytes for ledger comparison.

use std::fmt;

use crate::Error;

macro_rules! fixed_id {
    ($name:ident, $label:literal) => {
        /// A validated 16-byte identity.
        #[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
        pub struct $name([u8; 16]);

        impl $name {
            /// Validates one identity.
            ///
            /// # Errors
            ///
            /// Returns [`Error::InvalidId`] for the all-zero value.
            pub fn new(bytes: [u8; 16]) -> Result<Self, Error> {
                if bytes.iter().all(|byte| *byte == 0) {
                    Err(Error::InvalidId($label))
                } else {
                    Ok(Self(bytes))
                }
            }

            /// Returns the exact bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            /// Returns the first four bytes as eight lowercase hex characters for kernel names.
            #[must_use]
            pub fn short_hex(&self) -> String {
                self.0[..4]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({}..)", $label, self.short_hex())
            }
        }
    };
}

fixed_id!(BundleId, "bundle");
fixed_id!(InstanceId, "instance");
fixed_id!(OperationId, "operation");

/// The cleanup generation of one bundle; nonzero and monotonic per bundle identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct CleanupGeneration(u32);

impl CleanupGeneration {
    /// Validates one nonzero generation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidId`] for zero.
    pub fn new(value: u32) -> Result<Self, Error> {
        if value == 0 {
            Err(Error::InvalidId("generation"))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One conntrack zone identifier; zone zero is the shared default and never assigned.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ConntrackZone(u16);

impl ConntrackZone {
    /// Validates one nonzero zone.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidId`] for zero.
    pub fn new(value: u16) -> Result<Self, Error> {
        if value == 0 {
            Err(Error::InvalidId("zone"))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_reject_zero_and_expose_bytes() {
        assert_eq!(BundleId::new([0; 16]), Err(Error::InvalidId("bundle")));
        assert_eq!(InstanceId::new([0; 16]), Err(Error::InvalidId("instance")));
        assert_eq!(
            OperationId::new([0; 16]),
            Err(Error::InvalidId("operation"))
        );
        let id = BundleId::new([0xab, 0xcd, 0x01, 0x02, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 1])
            .expect("nonzero");
        assert_eq!(id.short_hex(), "abcd0102");
        assert_eq!(format!("{id:?}"), "bundle(abcd0102..)");
        assert_eq!(
            CleanupGeneration::new(0),
            Err(Error::InvalidId("generation"))
        );
        assert_eq!(ConntrackZone::new(0), Err(Error::InvalidId("zone")));
        assert_eq!(ConntrackZone::new(7).expect("nonzero").get(), 7);
    }
}
