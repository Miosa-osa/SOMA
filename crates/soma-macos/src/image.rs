use std::fmt;

use serde::Serialize;

use crate::{BackendError, ImageResolutionFailure};

/// Disclosure policy for the caller-supplied mutable image reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSourceReference {
    /// The reference is deliberately omitted from receipts and diagnostic serialization.
    Redacted,
}

/// Strength of the relationship between the observed manifest and a later launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageBinding {
    /// Digests were observed after pull, but Apple container cannot launch them immutably.
    ObservedOnlyNotEnforced,
}

#[derive(Clone, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ContentDigest(String);

impl ContentDigest {
    pub(crate) fn parse(
        value: impl Into<String>,
        failure: ImageResolutionFailure,
    ) -> Result<Self, BackendError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(BackendError::ImageResolution { failure });
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(BackendError::ImageResolution { failure });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ContentDigest")
            .field(&self.0)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ImagePlatform {
    os: String,
    architecture: String,
    variant: Option<String>,
}

impl ImagePlatform {
    pub(crate) const fn new(os: String, architecture: String, variant: Option<String>) -> Self {
        Self {
            os,
            architecture,
            variant,
        }
    }

    #[must_use]
    pub fn os(&self) -> &str {
        &self.os
    }

    #[must_use]
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    #[must_use]
    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ImageResolutionTimings {
    pull_millis: u64,
    inspect_millis: u64,
}

impl ImageResolutionTimings {
    pub(crate) const fn new(pull_millis: u64, inspect_millis: u64) -> Self {
        Self {
            pull_millis,
            inspect_millis,
        }
    }

    #[must_use]
    pub const fn pull_millis(self) -> u64 {
        self.pull_millis
    }

    #[must_use]
    pub const fn inspect_millis(self) -> u64 {
        self.inspect_millis
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedImage {
    source_reference: ImageSourceReference,
    index_digest: ContentDigest,
    manifest_digest: ContentDigest,
    platform: ImagePlatform,
    binding: ImageBinding,
    timings: ImageResolutionTimings,
}

impl ResolvedImage {
    pub(crate) const fn new(
        index_digest: ContentDigest,
        manifest_digest: ContentDigest,
        platform: ImagePlatform,
        timings: ImageResolutionTimings,
    ) -> Self {
        Self {
            source_reference: ImageSourceReference::Redacted,
            index_digest,
            manifest_digest,
            platform,
            binding: ImageBinding::ObservedOnlyNotEnforced,
            timings,
        }
    }

    #[must_use]
    pub const fn source_reference(&self) -> ImageSourceReference {
        self.source_reference
    }

    #[must_use]
    pub const fn index_digest(&self) -> &ContentDigest {
        &self.index_digest
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn platform(&self) -> &ImagePlatform {
        &self.platform
    }

    #[must_use]
    pub const fn binding(&self) -> ImageBinding {
        self.binding
    }

    #[must_use]
    pub const fn timings(&self) -> ImageResolutionTimings {
        self.timings
    }
}
