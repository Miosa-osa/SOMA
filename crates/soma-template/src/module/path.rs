//! Validated guest paths and environment names used by module and Template contracts.

use std::{error::Error, fmt};

pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_ENVIRONMENT_NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathError {
    Empty,
    NotAbsolute,
    NotNormalized,
    ForbiddenCharacter,
    TooLong,
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "path is empty",
            Self::NotAbsolute => "path is not absolute",
            Self::NotNormalized => "path is not normalized",
            Self::ForbiddenCharacter => "path contains a forbidden character",
            Self::TooLong => "path is too long",
        })
    }
}

impl Error for PathError {}

/// An absolute, normalized guest filesystem path.
///
/// The path never contains empty, `.`, or `..` segments, a NUL or control byte, or a
/// trailing slash other than the root itself, so ownership comparisons are exact.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GuestPath(String);

impl GuestPath {
    /// Parses one absolute normalized guest path.
    ///
    /// # Errors
    ///
    /// Returns a [`PathError`] for an empty, relative, oversized, unnormalized, or
    /// control-character-bearing value.
    pub fn parse(value: &str) -> Result<Self, PathError> {
        if value.is_empty() {
            return Err(PathError::Empty);
        }
        if value.len() > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        if value.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(PathError::ForbiddenCharacter);
        }
        if !value.starts_with('/') {
            return Err(PathError::NotAbsolute);
        }
        if value == "/" {
            return Ok(Self(value.to_owned()));
        }
        let normalized = value[1..]
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if !normalized {
            return Err(PathError::NotNormalized);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `self` is `other` or one of its ancestor directories.
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        if self.0 == "/" || self.0 == other.0 {
            return true;
        }
        other
            .0
            .strip_prefix(self.0.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// The final path segment, or an empty string for the root.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or_default()
    }
}

impl fmt::Display for GuestPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameError {
    Empty,
    TooLong,
    ForbiddenCharacter,
}

impl fmt::Display for NameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "name is empty",
            Self::TooLong => "name is too long",
            Self::ForbiddenCharacter => "name contains a forbidden character",
        })
    }
}

impl Error for NameError {}

/// A portable environment variable name: an ASCII letter or underscore followed by ASCII
/// letters, digits, or underscores.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    /// Parses one environment variable name.
    ///
    /// # Errors
    ///
    /// Returns a [`NameError`] for an empty, oversized, or non-portable name.
    pub fn parse(value: &str) -> Result<Self, NameError> {
        let mut bytes = value.bytes();
        let Some(first) = bytes.next() else {
            return Err(NameError::Empty);
        };
        if value.len() > MAX_ENVIRONMENT_NAME_BYTES {
            return Err(NameError::TooLong);
        }
        if !(first.is_ascii_alphabetic() || first == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(NameError::ForbiddenCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvironmentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
