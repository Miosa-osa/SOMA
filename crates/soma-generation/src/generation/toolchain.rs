//! The sealed builder environment: every external tool that materially shaped an artifact.
//!
//! A Generation is only as reproducible as the tools that produced it, so each formatter,
//! checker, populator, and inspector the compiler executes is bound by the digest of the exact
//! executable that ran and by the revision that executable reported about itself.
//! The ordered set is hashed into one builder-environment digest that the manifest carries and
//! that verification requires, so a Generation names the complete tool identity of its build
//! rather than one formatter and a version string.

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

#[cfg(test)]
mod tests;

const DOMAIN: &[u8] = b"soma-builder-environment-v1\0";
/// Largest number of tools one build may bind.
pub const MAX_BOUND_TOOLS: usize = 32;
/// Largest byte length of a bound tool's name or reported revision.
pub const MAX_TOOL_FIELD_BYTES: usize = 256;

/// One external tool bound to the exact bytes that executed.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BoundTool {
    name: String,
    digest: Sha256Digest,
    revision: String,
}

impl BoundTool {
    /// Binds one tool name, executable digest, and reported revision.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::InvalidInput`] for an empty or oversized name, a name that
    /// is not a bare file name, an oversized revision, or a control byte in either field.
    pub fn new(
        name: &str,
        digest: Sha256Digest,
        revision: &str,
        phase: CompilePhase,
    ) -> Result<Self, CompileError> {
        let named = !name.is_empty()
            && name.len() <= MAX_TOOL_FIELD_BYTES
            && !name.contains('/')
            && name != "."
            && name != "..";
        let described = revision.len() <= MAX_TOOL_FIELD_BYTES;
        let printable = name
            .chars()
            .chain(revision.chars())
            .all(|character| !character.is_control());
        if !named || !described || !printable {
            return Err(CompileError::new(phase, CompileErrorKind::InvalidInput));
        }
        Ok(Self {
            name: name.to_owned(),
            digest,
            revision: revision.to_owned(),
        })
    }

    /// Returns the tool's bare file name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the digest of the executable that ran.
    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    /// Returns the revision the executable reported about itself.
    #[must_use]
    pub fn revision(&self) -> &str {
        &self.revision
    }
}

/// The complete set of tools one build executed, kept in canonical name order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuilderEnvironment {
    tools: Vec<BoundTool>,
}

impl BuilderEnvironment {
    /// Returns an environment with nothing bound yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Binds one tool, or accepts an exact restatement of one already bound.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::Integrity`] when a name is bound twice to different bytes
    /// or a different revision, and [`CompileErrorKind::LimitExceeded`] beyond
    /// [`MAX_BOUND_TOOLS`].
    pub fn bind(&mut self, tool: BoundTool, phase: CompilePhase) -> Result<(), CompileError> {
        match self
            .tools
            .binary_search_by(|bound| bound.name.cmp(&tool.name))
        {
            Ok(index) => {
                if self.tools[index] == tool {
                    Ok(())
                } else {
                    Err(CompileError::new(phase, CompileErrorKind::Integrity))
                }
            }
            Err(index) if self.tools.len() < MAX_BOUND_TOOLS => {
                self.tools.insert(index, tool);
                Ok(())
            }
            Err(_) => Err(CompileError::new(phase, CompileErrorKind::LimitExceeded)),
        }
    }

    /// Binds every tool of another environment into this one.
    ///
    /// # Errors
    ///
    /// Returns the first conflict or limit failure reported by [`Self::bind`].
    pub fn absorb(&mut self, other: &Self, phase: CompilePhase) -> Result<(), CompileError> {
        for tool in &other.tools {
            self.bind(tool.clone(), phase)?;
        }
        Ok(())
    }

    /// Returns the bound tools in canonical name order.
    #[must_use]
    pub fn tools(&self) -> &[BoundTool] {
        &self.tools
    }

    /// Returns the digest over the canonical serialization of the bound tools.
    ///
    /// The serialization is domain separated, carries the tool count, and length-prefixes every
    /// variable field, so two different tool sets cannot produce the same bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::InvalidInput`] for an environment that bound no tool, which
    /// is never a build that produced an artifact.
    pub fn digest(&self, phase: CompilePhase) -> Result<Sha256Digest, CompileError> {
        if self.tools.is_empty() {
            return Err(CompileError::new(phase, CompileErrorKind::InvalidInput));
        }
        let count =
            u32::try_from(self.tools.len()).map_err(|_| CompileError::new(phase, invalid()))?;
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN);
        hasher.update(count.to_be_bytes());
        for tool in &self.tools {
            hasher.update(field_length(tool.name.len(), phase)?);
            hasher.update(tool.name.as_bytes());
            hasher.update(tool.digest.as_bytes());
            hasher.update(field_length(tool.revision.len(), phase)?);
            hasher.update(tool.revision.as_bytes());
        }
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(hasher.finalize().as_ref());
        Ok(Sha256Digest::from_bytes(digest))
    }
}

fn field_length(length: usize, phase: CompilePhase) -> Result<[u8; 2], CompileError> {
    u16::try_from(length)
        .map(u16::to_be_bytes)
        .map_err(|_| CompileError::new(phase, invalid()))
}

const fn invalid() -> CompileErrorKind {
    CompileErrorKind::InvalidInput
}
