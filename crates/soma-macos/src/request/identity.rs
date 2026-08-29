use std::fmt;

use serde::Serialize;

use super::{RequestError, RequestErrorReason};

#[derive(Clone, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct InstanceId(String);

impl InstanceId {
    /// Creates a stable caller-owned Instance identifier.
    ///
    /// # Errors
    ///
    /// Returns an error unless `value` contains exactly 32 lowercase hexadecimal characters.
    pub fn new(value: impl Into<String>) -> Result<Self, RequestError> {
        let value = value.into();
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RequestError::new(
                "instance_id",
                RequestErrorReason::InvalidIdentifier,
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub(crate) fn container_name(&self) -> String {
        format!("soma-{}", self.0)
    }

    #[must_use]
    pub(crate) fn ownership_label(&self) -> String {
        format!("io.miosa.soma.instance={}", self.0)
    }
}

impl fmt::Debug for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("InstanceId").field(&self.0).finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ImageReference(String);

impl ImageReference {
    /// Creates a validated OCI image reference.
    ///
    /// # Errors
    ///
    /// Returns an error unless the reference contains 1 to 1024 bytes and excludes option-like,
    /// URI-like, platform-dependent, whitespace, and control syntax.
    pub fn new(value: impl Into<String>) -> Result<Self, RequestError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RequestError::new("image", RequestErrorReason::Empty));
        }
        if value.len() > 1_024 {
            return Err(RequestError::new("image", RequestErrorReason::TooLarge));
        }
        if value.starts_with('-')
            || value.contains("://")
            || value.contains('\\')
            || value.chars().any(char::is_whitespace)
            || value.chars().any(char::is_control)
        {
            return Err(RequestError::new(
                "image",
                RequestErrorReason::InvalidCharacter,
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ImageReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageReference")
            .field("bytes", &self.0.len())
            .finish()
    }
}
