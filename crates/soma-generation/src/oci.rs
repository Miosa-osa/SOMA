use serde::{Deserialize, Deserializer};
use soma::{OciDigest, OciPlatform};

use crate::{ImportError, ImportErrorKind, ImportPhase, digest};

pub(crate) const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub(crate) const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub(crate) const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";
pub(crate) const PLAIN_LAYER: &str = "application/vnd.oci.image.layer.v1.tar";
pub(crate) const GZIP_LAYER: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

#[derive(Clone, Debug)]
pub(crate) struct Descriptor {
    pub(crate) media_type: String,
    pub(crate) digest: OciDigest,
    pub(crate) size: u64,
    pub(crate) platform: Option<OciPlatform>,
}

#[derive(Deserialize)]
pub(crate) struct LayoutWire {
    #[serde(rename = "imageLayoutVersion")]
    pub(crate) version: String,
}

#[derive(Deserialize)]
pub(crate) struct IndexWire {
    #[serde(rename = "schemaVersion")]
    pub(crate) schema_version: u32,
    #[serde(
        rename = "mediaType",
        default,
        deserialize_with = "deserialize_index_media_type"
    )]
    media_type: OptionalIndexMediaType,
    pub(crate) manifests: Vec<DescriptorWire>,
}

#[derive(Default)]
enum OptionalIndexMediaType {
    #[default]
    Absent,
    Present(String),
}

impl IndexWire {
    pub(crate) fn media_type_is_supported(&self) -> bool {
        match &self.media_type {
            OptionalIndexMediaType::Absent => true,
            OptionalIndexMediaType::Present(media_type) => media_type == INDEX_MEDIA_TYPE,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ManifestWire {
    #[serde(rename = "schemaVersion")]
    pub(crate) schema_version: u32,
    #[serde(rename = "mediaType")]
    pub(crate) media_type: Option<String>,
    pub(crate) config: DescriptorWire,
    pub(crate) layers: Vec<DescriptorWire>,
}

#[derive(Deserialize)]
pub(crate) struct ConfigWire {
    pub(crate) architecture: String,
    pub(crate) os: String,
    pub(crate) variant: Option<String>,
    #[serde(rename = "os.version")]
    os_version: Option<String>,
    #[serde(rename = "os.features")]
    os_features: Option<Vec<String>>,
    pub(crate) rootfs: RootFsWire,
}

#[derive(Deserialize)]
pub(crate) struct RootFsWire {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) diff_ids: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct DescriptorWire {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: i64,
    platform: Option<PlatformWire>,
}

#[derive(Deserialize)]
struct PlatformWire {
    os: String,
    architecture: String,
    variant: Option<String>,
    #[serde(rename = "os.version")]
    os_version: Option<String>,
    #[serde(rename = "os.features")]
    os_features: Option<Vec<String>>,
}

impl ConfigWire {
    pub(crate) fn require_supported_platform(&self) -> Result<(), ImportError> {
        require_supported_platform(
            self.os_version.as_deref(),
            self.os_features.as_deref(),
            ImportPhase::VerifyConfig,
        )
    }
}

impl DescriptorWire {
    pub(crate) fn media_type(&self) -> &str {
        &self.media_type
    }

    pub(crate) fn validate(self, phase: ImportPhase) -> Result<Descriptor, ImportError> {
        let size = u64::try_from(self.size)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::InvalidInput))?;
        let platform = self
            .platform
            .map(|value| {
                require_supported_platform(
                    value.os_version.as_deref(),
                    value.os_features.as_deref(),
                    phase,
                )?;
                OciPlatform::new(value.os, value.architecture, value.variant)
                    .map_err(|_| ImportError::new(phase, ImportErrorKind::InvalidInput))
            })
            .transpose()?;
        Ok(Descriptor {
            media_type: self.media_type,
            digest: digest::parse(self.digest, phase)?,
            size,
            platform,
        })
    }
}

fn require_supported_platform(
    os_version: Option<&str>,
    os_features: Option<&[String]>,
    phase: ImportPhase,
) -> Result<(), ImportError> {
    if os_version.is_some() || os_features.is_some_and(|features| !features.is_empty()) {
        return Err(ImportError::new(phase, ImportErrorKind::Unsupported));
    }
    Ok(())
}

pub(crate) fn merge_platform_claims(
    effective: &OciPlatform,
    declared: &OciPlatform,
    phase: ImportPhase,
) -> Result<OciPlatform, ImportError> {
    let same_base = declared.operating_system() == effective.operating_system()
        && declared.architecture() == effective.architecture();
    let variants_conflict = declared
        .variant()
        .zip(effective.variant())
        .is_some_and(|(left, right)| left != right);
    if !same_base || variants_conflict {
        return Err(ImportError::new(phase, ImportErrorKind::Integrity));
    }
    OciPlatform::new(
        effective.operating_system(),
        effective.architecture(),
        declared
            .variant()
            .or(effective.variant())
            .map(str::to_owned),
    )
    .map_err(|_| ImportError::new(phase, ImportErrorKind::Integrity))
}

fn deserialize_index_media_type<'de, D>(deserializer: D) -> Result<OptionalIndexMediaType, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(OptionalIndexMediaType::Present)
}

pub(crate) fn parse_json<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    phase: ImportPhase,
) -> Result<T, ImportError> {
    serde_json::from_slice(bytes)
        .map_err(|_| ImportError::new(phase, ImportErrorKind::InvalidInput))
}
