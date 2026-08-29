use serde::{Deserialize, Serialize};

use crate::{GenerationId, OciDigest, OciPlatform, RequestFingerprint};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadIdentity {
    #[serde(default)]
    index_digest: Option<OciDigest>,
    manifest_digest: OciDigest,
    platform: OciPlatform,
    generation_id: Option<GenerationId>,
}

impl WorkloadIdentity {
    #[must_use]
    pub const fn new(
        manifest_digest: OciDigest,
        platform: OciPlatform,
        generation_id: Option<GenerationId>,
    ) -> Self {
        Self {
            index_digest: None,
            manifest_digest,
            platform,
            generation_id,
        }
    }

    #[must_use]
    pub fn with_index_digest(mut self, index_digest: OciDigest) -> Self {
        self.index_digest = Some(index_digest);
        self
    }

    #[must_use]
    pub fn index_digest(&self) -> Option<&OciDigest> {
        self.index_digest.as_ref()
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &OciDigest {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn platform(&self) -> &OciPlatform {
        &self.platform
    }

    #[must_use]
    pub fn generation_id(&self) -> Option<&GenerationId> {
        self.generation_id.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkloadEvidence {
    Resolved {
        identity: WorkloadIdentity,
    },
    Unresolved {
        source_fingerprint: RequestFingerprint,
    },
}
