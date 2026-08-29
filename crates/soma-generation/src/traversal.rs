use soma::{OciDigest, OciPlatform, WorkloadIdentity};

use crate::{
    ImportError, ImportErrorKind, ImportLimits, ImportPhase, OciSelection, digest,
    layout::Layout,
    oci::{
        Descriptor, INDEX_MEDIA_TYPE, IndexWire, MANIFEST_MEDIA_TYPE, merge_platform_claims,
        parse_json,
    },
};

const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INDEX_DEPTH: usize = 8;

#[derive(Clone)]
pub(crate) struct Candidate {
    pub(crate) manifest: Descriptor,
    pub(crate) indexes: Vec<Descriptor>,
    pub(crate) declared_platform: Option<OciPlatform>,
}

pub(crate) struct Traversal {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) top_index_bytes: Vec<u8>,
    pub(crate) top_index_digest: OciDigest,
    pub(crate) descriptor_count: u32,
}

pub(crate) fn traverse(
    layout: &Layout,
    selection: OciSelection<'_>,
    limits: ImportLimits,
) -> Result<Traversal, ImportError> {
    let top = layout.read_top_index(limits.max_blob_bytes)?;
    let index: IndexWire = parse_json(&top, ImportPhase::SelectManifest)?;
    require_index(&index)?;
    let mut state = State::new(selection, limits);
    state.walk(layout, index, &[], 0)?;
    Ok(Traversal {
        candidates: state.candidates,
        top_index_digest: digest::bytes(&top),
        top_index_bytes: top,
        descriptor_count: state.descriptors,
    })
}

struct State<'a> {
    selection: OciSelection<'a>,
    limits: ImportLimits,
    descriptors: u32,
    candidates: Vec<Candidate>,
}

impl<'a> State<'a> {
    const fn new(selection: OciSelection<'a>, limits: ImportLimits) -> Self {
        Self {
            selection,
            limits,
            descriptors: 0,
            candidates: Vec::new(),
        }
    }

    fn walk(
        &mut self,
        layout: &Layout,
        index: IndexWire,
        path: &[Descriptor],
        depth: usize,
    ) -> Result<(), ImportError> {
        if depth >= MAX_INDEX_DEPTH {
            return Err(limit_error());
        }
        for wire in index.manifests {
            self.descriptors = self.descriptors.checked_add(1).ok_or_else(limit_error)?;
            if self.descriptors > self.limits.max_descriptors {
                return Err(limit_error());
            }
            if !matches!(wire.media_type(), INDEX_MEDIA_TYPE | MANIFEST_MEDIA_TYPE) {
                continue;
            }
            let descriptor = wire.validate(ImportPhase::SelectManifest)?;
            match descriptor.media_type.as_str() {
                INDEX_MEDIA_TYPE => self.walk_index(layout, descriptor, path, depth)?,
                MANIFEST_MEDIA_TYPE => {
                    let declared = declared_platform(path, &descriptor)?;
                    if self.may_match(&descriptor, declared.as_ref()) {
                        self.add_candidate(descriptor, path, declared)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn walk_index(
        &mut self,
        layout: &Layout,
        descriptor: Descriptor,
        path: &[Descriptor],
        depth: usize,
    ) -> Result<(), ImportError> {
        reject_cycle(path, &descriptor)?;
        let bytes = layout.read_blob(
            &descriptor,
            MAX_METADATA_BYTES.min(self.limits.max_blob_bytes),
            ImportPhase::SelectManifest,
        )?;
        let nested: IndexWire = parse_json(&bytes, ImportPhase::SelectManifest)?;
        require_index(&nested)?;
        let mut nested_path = path.to_vec();
        nested_path.push(descriptor);
        self.walk(layout, nested, &nested_path, depth + 1)
    }

    fn add_candidate(
        &mut self,
        descriptor: Descriptor,
        path: &[Descriptor],
        declared_platform: Option<OciPlatform>,
    ) -> Result<(), ImportError> {
        if let Some(existing) = self
            .candidates
            .iter()
            .find(|candidate| candidate.manifest.digest == descriptor.digest)
        {
            if existing.manifest.media_type != descriptor.media_type
                || existing.manifest.size != descriptor.size
                || existing.declared_platform != declared_platform
            {
                return Err(ImportError::new(
                    ImportPhase::SelectManifest,
                    ImportErrorKind::Integrity,
                ));
            }
            return Ok(());
        }
        self.candidates.push(Candidate {
            manifest: descriptor,
            indexes: path.to_vec(),
            declared_platform,
        });
        Ok(())
    }

    fn may_match(&self, descriptor: &Descriptor, declared: Option<&OciPlatform>) -> bool {
        match self.selection {
            OciSelection::Exact(identity) => descriptor.digest == *identity.manifest_digest(),
            OciSelection::Platform(platform) => {
                declared.is_none_or(|actual| platform_may_match(platform, actual))
            }
        }
    }
}

fn declared_platform(
    path: &[Descriptor],
    manifest: &Descriptor,
) -> Result<Option<OciPlatform>, ImportError> {
    let mut effective = None;
    for declared in path
        .iter()
        .filter_map(|descriptor| descriptor.platform.as_ref())
        .chain(manifest.platform.as_ref())
    {
        effective = Some(match &effective {
            Some(current) => merge_platform_claims(current, declared, ImportPhase::SelectManifest)?,
            None => declared.clone(),
        });
    }
    Ok(effective)
}

pub(crate) fn platform_matches(requested: &OciPlatform, actual: &OciPlatform) -> bool {
    requested.operating_system() == actual.operating_system()
        && requested.architecture() == actual.architecture()
        && requested
            .variant()
            .is_none_or(|variant| actual.variant() == Some(variant))
}

fn platform_may_match(requested: &OciPlatform, declared: &OciPlatform) -> bool {
    requested.operating_system() == declared.operating_system()
        && requested.architecture() == declared.architecture()
        && requested
            .variant()
            .zip(declared.variant())
            .is_none_or(|(left, right)| left == right)
}

fn require_index(index: &IndexWire) -> Result<(), ImportError> {
    if index.schema_version != 2 || !index.media_type_is_supported() {
        return Err(ImportError::new(
            ImportPhase::SelectManifest,
            ImportErrorKind::Unsupported,
        ));
    }
    Ok(())
}

fn reject_cycle(path: &[Descriptor], descriptor: &Descriptor) -> Result<(), ImportError> {
    if path
        .iter()
        .any(|ancestor| ancestor.digest == descriptor.digest)
    {
        return Err(ImportError::new(
            ImportPhase::SelectManifest,
            ImportErrorKind::InvalidInput,
        ));
    }
    Ok(())
}

pub(crate) fn expected_identity(selection: OciSelection<'_>) -> Option<&WorkloadIdentity> {
    match selection {
        OciSelection::Exact(identity) => Some(identity),
        OciSelection::Platform(_) => None,
    }
}

const fn limit_error() -> ImportError {
    ImportError::new(ImportPhase::SelectManifest, ImportErrorKind::LimitExceeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_node_cycle_is_rejected_before_reopening_an_ancestor() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("layout");
        std::fs::create_dir_all(path.join("blobs/sha256")).unwrap();
        std::fs::write(
            path.join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )
        .unwrap();
        let layout = Layout::open(&path, ImportLimits::default().max_blob_bytes).unwrap();
        let platform = OciPlatform::linux_arm64();
        let mut state = State::new(OciSelection::Platform(&platform), ImportLimits::default());
        let first = index_descriptor('a');
        let second = index_descriptor('b');

        let error = state
            .walk_index(&layout, first.clone(), &[first, second], 2)
            .unwrap_err();

        assert_eq!(error.kind(), ImportErrorKind::InvalidInput);
    }

    fn index_descriptor(hex: char) -> Descriptor {
        Descriptor {
            media_type: INDEX_MEDIA_TYPE.to_owned(),
            digest: OciDigest::parse(format!("sha256:{}", hex.to_string().repeat(64))).unwrap(),
            size: 1,
            platform: None,
        }
    }
}
