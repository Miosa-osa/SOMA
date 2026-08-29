//! The required validation of a composed Template against external inputs.
//!
//! Validation runs in one fixed order so the same Template always reports the same first
//! rejection: platforms, resources, lifecycle, description, command shape, module values,
//! environment, network envelope, secrets, required environment, and finally the executable
//! check.
//! Secret-literal detection runs wherever a Template or module literal is bound: environment
//! values, command fields, the description, secret sources and scopes, and sealed values.

mod backend;
mod checks;
pub(crate) mod cidr;
mod contract;
mod network;
pub(crate) mod policy;
mod secret;
pub(crate) mod syntax;

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{
    compose::Composition,
    error::TemplateError,
    lock::{LockedCommand, LockedEnvironment, LockedSecret},
    resolve::ResolvedImage,
    schema::Template,
};

pub use backend::{BackendCapabilities, MAX_BACKEND_PLATFORMS, ResourceLimits};
pub use network::{EgressEnvelope, NetworkEnvelope};
pub use policy::PolicyCeiling;

/// An infrastructure failure while answering a filesystem question.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleError {
    detail: String,
}

impl OracleError {
    #[must_use]
    pub fn new(detail: &str) -> Self {
        Self {
            detail: detail.to_owned(),
        }
    }
}

impl fmt::Display for OracleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for OracleError {}

/// Answers whether the resolved base filesystem provides an executable.
///
/// A real oracle inspects the normalized rootfs of the resolved image; the test oracle
/// answers from a table.
/// A program without a slash is looked up by name, and an absolute program by exact path.
pub trait FilesystemOracle {
    /// # Errors
    ///
    /// Returns [`OracleError`] when the filesystem could not be consulted at all.
    fn executable_present(&self, image: &ResolvedImage, program: &str)
    -> Result<bool, OracleError>;
}

/// A deterministic in-memory oracle keyed by image digest.
#[derive(Clone, Debug, Default)]
pub struct TestFilesystemOracle {
    executables: BTreeMap<String, BTreeSet<String>>,
}

impl TestFilesystemOracle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one absolute executable path present in the image with `digest`.
    #[must_use]
    pub fn with_executable(mut self, digest: &soma::OciDigest, path: &str) -> Self {
        self.executables
            .entry(digest.as_str().to_owned())
            .or_default()
            .insert(path.to_owned());
        self
    }
}

impl FilesystemOracle for TestFilesystemOracle {
    fn executable_present(
        &self,
        image: &ResolvedImage,
        program: &str,
    ) -> Result<bool, OracleError> {
        let Some(paths) = self.executables.get(image.digest().as_str()) else {
            return Ok(false);
        };
        if program.contains('/') {
            return Ok(paths.contains(program));
        }
        Ok(paths
            .iter()
            .any(|path| path.rsplit('/').next() == Some(program)))
    }
}

pub(crate) struct Validated {
    pub(crate) command: LockedCommand,
    pub(crate) network: NetworkEnvelope,
    pub(crate) environment: Vec<LockedEnvironment>,
    pub(crate) secrets: Vec<LockedSecret>,
}

pub(crate) fn validate(
    template: &Template,
    composition: &Composition<'_>,
    image: &ResolvedImage,
    ceiling: &PolicyCeiling,
    backend: &BackendCapabilities,
    oracle: &dyn FilesystemOracle,
) -> Result<Validated, TemplateError> {
    checks::platforms(template, composition, backend)?;
    checks::resources(template, backend)?;
    checks::lifecycle(template, backend)?;
    checks::description(template)?;
    let command = checks::command(&composition.command)?;
    checks::modules(composition)?;
    let environment = contract::environment(template, composition)?;
    let network = network::envelope(template.network(), ceiling)?;
    let secrets = contract::secrets(template, &network)?;
    contract::required_environment(composition, &environment, &secrets)?;
    checks::executable(&command, &composition.modules, image, oracle)?;
    Ok(Validated {
        command,
        network,
        environment,
        secrets,
    })
}
