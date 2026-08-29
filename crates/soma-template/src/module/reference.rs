//! `soma://<kind>/<name>@<version>` module references.

use std::{error::Error, fmt};

use super::{MAX_MODULE_NAME_BYTES, ModuleIdentity, ModuleKind};

const SCHEME: &str = "soma://";
const MAX_REFERENCE_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleRefError {
    TooLong,
    Scheme,
    Kind,
    Name,
    Version,
}

impl fmt::Display for ModuleRefError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLong => "module reference is too long",
            Self::Scheme => "module reference must start with soma://",
            Self::Kind => "module reference has an unknown kind",
            Self::Name => "module reference has an invalid name",
            Self::Version => "module reference has an invalid version",
        })
    }
}

impl Error for ModuleRefError {}

/// A module reference that may still be unpinned.
///
/// A pinned reference names exactly one immutable module version; an unpinned reference is
/// an authoring shortcut that composition rejects.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ModuleRef {
    kind: ModuleKind,
    name: String,
    version: Option<u32>,
}

impl ModuleRef {
    /// Parses `soma://<kind>/<name>` with an optional `@<version>` suffix.
    ///
    /// # Errors
    ///
    /// Returns a [`ModuleRefError`] for a wrong scheme, unknown kind, non-portable name,
    /// or a version that is not a decimal `u32`.
    pub fn parse(value: &str) -> Result<Self, ModuleRefError> {
        if value.len() > MAX_REFERENCE_BYTES {
            return Err(ModuleRefError::TooLong);
        }
        let rest = value.strip_prefix(SCHEME).ok_or(ModuleRefError::Scheme)?;
        let (kind, rest) = rest.split_once('/').ok_or(ModuleRefError::Kind)?;
        let kind = ModuleKind::parse(kind).ok_or(ModuleRefError::Kind)?;
        let (name, version) = match rest.split_once('@') {
            Some((name, version)) => (name, Some(version)),
            None => (rest, None),
        };
        if !valid_name(name) {
            return Err(ModuleRefError::Name);
        }
        let version = match version {
            Some(text) => Some(parse_version(text)?),
            None => None,
        };
        Ok(Self {
            kind,
            name: name.to_owned(),
            version,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> ModuleKind {
        self.kind
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> Option<u32> {
        self.version
    }

    /// The exact identity when the reference is pinned.
    #[must_use]
    pub fn pinned(&self) -> Option<ModuleIdentity> {
        self.version
            .map(|version| ModuleIdentity::unchecked(self.kind, self.name.clone(), version))
    }
}

impl From<ModuleIdentity> for ModuleRef {
    fn from(identity: ModuleIdentity) -> Self {
        Self {
            kind: identity.kind(),
            name: identity.name().to_owned(),
            version: Some(identity.version()),
        }
    }
}

impl fmt::Display for ModuleRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{SCHEME}{}/{}", self.kind.as_str(), self.name)?;
        if let Some(version) = self.version {
            write!(formatter, "@{version}")?;
        }
        Ok(())
    }
}

fn parse_version(text: &str) -> Result<u32, ModuleRefError> {
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ModuleRefError::Version);
    }
    if text.len() > 1 && text.starts_with('0') {
        return Err(ModuleRefError::Version);
    }
    text.parse().map_err(|_| ModuleRefError::Version)
}

/// A module name is lowercase ASCII letters, digits, and interior hyphens.
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MODULE_NAME_BYTES
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
