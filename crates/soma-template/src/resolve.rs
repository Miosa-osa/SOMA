//! Resolution: compose, pin the OCI digest, validate, and emit the Template Lock.

use std::{collections::BTreeMap, error::Error, fmt};

use soma::{OciDigest, OciImage, OciPlatform};

use crate::{
    compose,
    error::{ExternalDependency, LockError, TemplateError},
    lock::TemplateLock,
    module::ModuleRegistry,
    rejection::Rejection,
    schema::Template,
    validate::{self, BackendCapabilities, FilesystemOracle, PolicyCeiling, policy::platform_key},
    wire::{Reader, Writer},
};

/// Why a resolver did not return an exact digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolveError {
    /// The reference does not name an image for the requested platform.
    Unresolvable,
    /// The registry or cache could not be consulted.
    Unavailable(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolvable => formatter.write_str("image reference is unresolvable"),
            Self::Unavailable(detail) => write!(formatter, "resolver unavailable: {detail}"),
        }
    }
}

impl Error for ResolveError {}

/// One exact OCI manifest selected for one platform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedImage {
    digest: OciDigest,
    platform: OciPlatform,
    size: u64,
}

impl ResolvedImage {
    #[must_use]
    pub const fn new(digest: OciDigest, platform: OciPlatform, size: u64) -> Self {
        Self {
            digest,
            platform,
            size,
        }
    }

    #[must_use]
    pub const fn digest(&self) -> &OciDigest {
        &self.digest
    }

    #[must_use]
    pub const fn platform(&self) -> &OciPlatform {
        &self.platform
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_bytes(&digest_bytes(&self.digest));
        writer.put_u64(self.size);
        writer.put_string(self.platform.operating_system());
        writer.put_string(self.platform.architecture());
        writer.put_optional_string(self.platform.variant());
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, LockError> {
        let digest = digest_from_bytes(&reader.array::<32>()?);
        let size = reader.u64()?;
        let bound = crate::schema::MAX_STRING_BYTES;
        let operating_system = reader.string(bound)?;
        let architecture = reader.string(bound)?;
        let variant = reader.optional_string(bound)?;
        let platform = OciPlatform::new(operating_system, architecture, variant).map_err(|_| {
            LockError::InvalidField {
                field: "workload.platform",
            }
        })?;
        Ok(Self {
            digest,
            platform,
            size,
        })
    }
}

/// Resolves a mutable OCI reference to an exact manifest digest for one platform.
pub trait OciResolver {
    /// # Errors
    ///
    /// Returns [`ResolveError::Unresolvable`] when no exact digest exists for the reference
    /// and platform, or [`ResolveError::Unavailable`] for an infrastructure failure.
    fn resolve(
        &self,
        reference: &OciImage,
        platform: &OciPlatform,
    ) -> Result<ResolvedImage, ResolveError>;
}

/// A deterministic offline resolver backed by an explicit table.
#[derive(Clone, Debug, Default)]
pub struct TestResolver {
    images: BTreeMap<(String, String), (OciDigest, u64)>,
}

impl TestResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins one reference and platform to a digest and manifest size.
    #[must_use]
    pub fn with_image(
        mut self,
        reference: &str,
        platform: &OciPlatform,
        digest: OciDigest,
        size: u64,
    ) -> Self {
        self.images.insert(
            (reference.to_owned(), platform_key(platform)),
            (digest, size),
        );
        self
    }
}

impl OciResolver for TestResolver {
    fn resolve(
        &self,
        reference: &OciImage,
        platform: &OciPlatform,
    ) -> Result<ResolvedImage, ResolveError> {
        let key = (reference.as_str().to_owned(), platform_key(platform));
        let (digest, size) = self.images.get(&key).ok_or(ResolveError::Unresolvable)?;
        Ok(ResolvedImage::new(digest.clone(), platform.clone(), *size))
    }
}

/// Resolves a Template with the built-in module registry.
///
/// # Errors
///
/// Returns [`TemplateError::Rejected`] for any required validation failure and
/// [`TemplateError::Unavailable`] when the resolver or oracle could not answer.
pub fn resolve(
    template: &Template,
    resolver: &dyn OciResolver,
    ceiling: &PolicyCeiling,
    backend: &BackendCapabilities,
    oracle: &dyn FilesystemOracle,
) -> Result<TemplateLock, TemplateError> {
    resolve_with(
        &ModuleRegistry::builtin(),
        template,
        resolver,
        ceiling,
        backend,
        oracle,
    )
}

/// Resolves a Template against an explicit module registry.
///
/// # Errors
///
/// See [`resolve`].
pub fn resolve_with(
    registry: &ModuleRegistry,
    template: &Template,
    resolver: &dyn OciResolver,
    ceiling: &PolicyCeiling,
    backend: &BackendCapabilities,
    oracle: &dyn FilesystemOracle,
) -> Result<TemplateLock, TemplateError> {
    let composition = compose::compose(template, registry)?;
    let image = resolve_image(template, resolver)?;
    let validated = validate::validate(template, &composition, &image, ceiling, backend, oracle)?;
    Ok(TemplateLock::assemble(
        template,
        &composition,
        image,
        validated,
        ceiling,
        backend,
    ))
}

fn resolve_image(
    template: &Template,
    resolver: &dyn OciResolver,
) -> Result<ResolvedImage, TemplateError> {
    let workload = template.workload();
    let unresolvable = || Rejection::UnresolvableImage {
        field: "workload.image".to_owned(),
        reference: workload.image().as_str().to_owned(),
        platform: platform_key(workload.platform()),
    };
    let image = match resolver.resolve(workload.image(), workload.platform()) {
        Ok(image) => image,
        Err(ResolveError::Unresolvable) => return Err(unresolvable().into()),
        Err(ResolveError::Unavailable(detail)) => {
            return Err(TemplateError::Unavailable {
                dependency: ExternalDependency::OciResolver,
                detail,
            });
        }
    };
    if image.platform() != workload.platform() {
        return Err(unresolvable().into());
    }
    if let Some((_, pinned)) = workload.image().as_str().split_once('@')
        && pinned != image.digest().as_str()
    {
        return Err(unresolvable().into());
    }
    Ok(image)
}

pub(crate) fn digest_bytes(digest: &OciDigest) -> [u8; 32] {
    let hex = digest.as_str().strip_prefix("sha256:").unwrap_or_default();
    let mut output = [0_u8; 32];
    for (index, pair) in hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .take(32)
    {
        output[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    output
}

const fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

pub(crate) fn digest_from_bytes(bytes: &[u8; 32]) -> OciDigest {
    OciDigest::parse(format!("sha256:{}", hex(bytes)))
        .expect("32 hex-encoded bytes form a canonical OCI digest")
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
