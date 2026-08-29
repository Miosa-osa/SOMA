//! Backend capabilities supplied to validation and bound into the lock.

use soma::OciPlatform;

use super::policy::platform_key;
use crate::{
    error::{BoundError, LockError},
    schema::{IdleAction, MAX_STRING_BYTES},
    wire::{Reader, Writer},
};

pub const MAX_BACKEND_PLATFORMS: usize = 16;

/// The largest Machine shape a Backend admits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    pub max_vcpus: u32,
    pub max_memory_mib: u64,
    pub max_writable_storage_mib: u64,
}

/// What the selected Backend has proven it supports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendCapabilities {
    platforms: Vec<OciPlatform>,
    idle_actions: u8,
    limits: ResourceLimits,
}

impl BackendCapabilities {
    /// Creates one capability set; platforms are stored sorted and deduplicated.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError`] for an empty or oversized platform list.
    pub fn new(
        platforms: &[OciPlatform],
        idle_actions: &[IdleAction],
        limits: ResourceLimits,
    ) -> Result<Self, BoundError> {
        if platforms.is_empty() {
            return Err(BoundError::Empty {
                field: "backend.platforms".to_owned(),
            });
        }
        if platforms.len() > MAX_BACKEND_PLATFORMS {
            return Err(BoundError::TooMany {
                field: "backend.platforms".to_owned(),
                maximum: MAX_BACKEND_PLATFORMS,
            });
        }
        let mut keyed: Vec<(String, OciPlatform)> = platforms
            .iter()
            .map(|platform| (platform_key(platform), platform.clone()))
            .collect();
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        keyed.dedup_by(|left, right| left.0 == right.0);
        let mask = idle_actions
            .iter()
            .fold(0_u8, |mask, action| mask | (1 << action.code()));
        Ok(Self {
            platforms: keyed.into_iter().map(|(_, platform)| platform).collect(),
            idle_actions: mask,
            limits,
        })
    }

    #[must_use]
    pub fn platforms(&self) -> &[OciPlatform] {
        &self.platforms
    }

    #[must_use]
    pub fn supports_platform(&self, platform: &OciPlatform) -> bool {
        self.platforms.contains(platform)
    }

    #[must_use]
    pub const fn supports_idle_action(&self, action: IdleAction) -> bool {
        self.idle_actions & (1 << action.code()) != 0
    }

    #[must_use]
    pub fn idle_actions(&self) -> Vec<IdleAction> {
        IdleAction::ALL
            .into_iter()
            .filter(|action| self.supports_idle_action(*action))
            .collect()
    }

    #[must_use]
    pub const fn limits(&self) -> ResourceLimits {
        self.limits
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_count(self.platforms.len());
        for platform in &self.platforms {
            writer.put_string(platform.operating_system());
            writer.put_string(platform.architecture());
            writer.put_optional_string(platform.variant());
        }
        writer.put_u8(self.idle_actions);
        writer.put_u32(self.limits.max_vcpus);
        writer.put_u64(self.limits.max_memory_mib);
        writer.put_u64(self.limits.max_writable_storage_mib);
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, LockError> {
        const FIELD: &str = "backend.platforms";
        let count = reader.count(MAX_BACKEND_PLATFORMS)?;
        let mut platforms = Vec::with_capacity(count);
        for _ in 0..count {
            let operating_system = reader.string(MAX_STRING_BYTES)?;
            let architecture = reader.string(MAX_STRING_BYTES)?;
            let variant = reader.optional_string(MAX_STRING_BYTES)?;
            platforms.push(
                OciPlatform::new(operating_system, architecture, variant)
                    .map_err(|_| LockError::InvalidField { field: FIELD })?,
            );
        }
        let idle_actions = reader.u8()?;
        if idle_actions & !0b111 != 0 {
            return Err(LockError::InvalidDiscriminant {
                field: "backend.idle_actions",
                value: idle_actions,
            });
        }
        let limits = ResourceLimits {
            max_vcpus: reader.u32()?,
            max_memory_mib: reader.u64()?,
            max_writable_storage_mib: reader.u64()?,
        };
        let keys: Vec<String> = platforms.iter().map(platform_key).collect();
        if keys.is_empty() || !keys.is_sorted() || keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(LockError::InvalidField { field: FIELD });
        }
        Ok(Self {
            platforms,
            idle_actions,
            limits,
        })
    }
}
