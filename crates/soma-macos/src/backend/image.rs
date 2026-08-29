use std::ffi::OsString;

use serde::Deserialize;

use crate::{
    BackendError, CommandFailure, CommandFailureReason, ContentDigest, ControlLimits,
    ImagePlatform, ImageReference, ImageResolutionFailure, ImageResolutionTimings, Operation,
    ResolvedImage,
};

use super::MacOsBackend;

const TARGET_PLATFORM: &str = "linux/arm64";
const TARGET_OS: &str = "linux";
const TARGET_ARCHITECTURE: &str = "arm64";
const TARGET_VARIANT: &str = "v8";

impl MacOsBackend {
    /// Pulls and resolves one image to exact Linux ARM64 index and manifest identities.
    ///
    /// Pull and inspect durations remain separate from launch-ready timing.
    /// Apple container 1.3 does not expose an immutable local digest launch reference, and its
    /// content-derived tags remain mutable, so this result deliberately does not claim an
    /// immutable launch alias.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the pull or inspect command fails, metadata is malformed, the
    /// inspected image is ambiguous, or its only variant is not Linux ARM64.
    pub fn resolve_image(
        &self,
        image: &ImageReference,
        limits: ControlLimits,
    ) -> Result<ResolvedImage, BackendError> {
        self.ensure_host()?;
        let pull = self
            .commands
            .execute_control(
                Operation::PullImage,
                strings(&[
                    "image",
                    "pull",
                    "--progress",
                    "none",
                    "--platform",
                    TARGET_PLATFORM,
                    image.as_str(),
                ]),
                limits,
            )
            .map_err(BackendError::command)?;
        require_success(Operation::PullImage, pull.status())?;

        let inspect = self
            .commands
            .execute_control(
                Operation::InspectImage,
                strings(&["image", "inspect", image.as_str()]),
                limits,
            )
            .map_err(BackendError::command)?;
        require_success(Operation::InspectImage, inspect.status())?;
        let identity = parse_inspection(inspect.stdout())?;

        Ok(ResolvedImage::new(
            identity.index_digest,
            identity.manifest_digest,
            identity.platform,
            ImageResolutionTimings::new(pull.elapsed_millis(), inspect.elapsed_millis()),
        ))
    }
}

struct ResolvedIdentity {
    index_digest: ContentDigest,
    manifest_digest: ContentDigest,
    platform: ImagePlatform,
}

#[derive(Deserialize)]
struct RawImageRecord {
    configuration: RawConfiguration,
    id: String,
    variants: Vec<RawVariant>,
}

#[derive(Deserialize)]
struct RawConfiguration {
    descriptor: RawDescriptor,
}

#[derive(Deserialize)]
struct RawDescriptor {
    digest: String,
}

#[derive(Deserialize)]
struct RawVariant {
    digest: String,
    platform: RawPlatform,
}

#[derive(Deserialize)]
struct RawPlatform {
    architecture: String,
    os: String,
    variant: Option<String>,
}

fn parse_inspection(document: &[u8]) -> Result<ResolvedIdentity, BackendError> {
    let records = serde_json::from_slice::<Vec<RawImageRecord>>(document).map_err(|_| {
        BackendError::ImageResolution {
            failure: ImageResolutionFailure::InvalidJson,
        }
    })?;
    let record = match records.as_slice() {
        [] => {
            return Err(resolution_failure(
                ImageResolutionFailure::MissingImageRecord,
            ));
        }
        [record] => record,
        _ => {
            return Err(resolution_failure(
                ImageResolutionFailure::MultipleImageRecords,
            ));
        }
    };
    let variant = match record.variants.as_slice() {
        [] => return Err(resolution_failure(ImageResolutionFailure::MissingVariant)),
        [variant] => variant,
        _ => return Err(resolution_failure(ImageResolutionFailure::MultipleVariants)),
    };
    if variant.platform.os != TARGET_OS
        || variant.platform.architecture != TARGET_ARCHITECTURE
        || variant.platform.variant.as_deref() != Some(TARGET_VARIANT)
    {
        return Err(resolution_failure(ImageResolutionFailure::PlatformMismatch));
    }

    let index_digest = ContentDigest::parse(
        record.configuration.descriptor.digest.clone(),
        ImageResolutionFailure::MalformedIndexDigest,
    )?;
    if index_digest.as_str().strip_prefix("sha256:") != Some(record.id.as_str()) {
        return Err(resolution_failure(
            ImageResolutionFailure::IndexIdentityMismatch,
        ));
    }
    let manifest_digest = ContentDigest::parse(
        variant.digest.clone(),
        ImageResolutionFailure::MalformedManifestDigest,
    )?;
    let platform = ImagePlatform::new(
        variant.platform.os.clone(),
        variant.platform.architecture.clone(),
        variant.platform.variant.clone(),
    );
    Ok(ResolvedIdentity {
        index_digest,
        manifest_digest,
        platform,
    })
}

fn require_success(
    operation: Operation,
    status: crate::ExecutionStatus,
) -> Result<(), BackendError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(BackendError::command(CommandFailure::new(
            operation,
            CommandFailureReason::Status(status),
        )))
    }
}

const fn resolution_failure(failure: ImageResolutionFailure) -> BackendError {
    BackendError::ImageResolution { failure }
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
