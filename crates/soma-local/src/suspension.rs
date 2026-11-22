//! Suspending a managed Machine into a sealed Suspension, and resuming one as a new Instance
//! (ADR 0034).
//!
//! A suspension reads the running machine into a staging directory, ends the Instance with a
//! `Suspended` receipt, and seals the three members under a fresh data key in this host's store;
//! the plaintext is removed once sealed. A resume decrypts the members, launches a new Instance
//! that restores them instead of its Generation's snapshot, and destroys the Suspension once that
//! Instance runs, so one Suspension resumes at most once.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use soma::{
    ExecutionReceipt, InstanceId, LaunchMachineRequest, MachineLaunch, MachineShape,
    ManagedFailure, OciImage, OperationId, SuspendMachineRequest, WorkloadEvidence,
};
use soma_hostd::suspension::store::{Store, StoreError};
use soma_hostd::suspension::{Header, MAX_RETENTION_SECS, MemberRecord, Shape};
use soma_hostd::{GenerationId, HostProfileDigest, SuspensionId};

/// The file a resume directory names its Generation in, which the launch checks before it
/// restores anything: members captured over one root must never run over another.
pub const RESUME_GENERATION_FILE: &str = "generation";

/// What a caller asks of a suspension.
#[derive(Clone, Debug)]
pub struct SuspendRequest {
    pub operation_id: OperationId,
    pub instance_id: InstanceId,
    /// Who owns the Suspension; it is bound into the sealed header.
    pub tenant: String,
    /// The image the Instance was launched from, kept so a resume need not name it again.
    pub image: Option<OciImage>,
    /// How long the Suspension is kept, capped at the store's ceiling.
    pub retention_secs: u64,
}

/// A sealed Suspension.
#[derive(Clone, Debug)]
pub struct Suspended {
    /// The Suspension's identity, 32 lowercase hexadecimal characters.
    pub suspension_id: String,
    pub expires_unix: u64,
    /// The plaintext bytes sealed: memory, private head and machine state.
    pub sealed_bytes: u64,
    /// The receipt that ended the Instance.
    pub receipt: ExecutionReceipt,
}

/// What a caller asks of a resume.
#[derive(Clone, Debug)]
pub struct ResumeRequest {
    pub operation_id: OperationId,
    /// The new Instance; a resume never reuses the suspended one's identity.
    pub instance_id: InstanceId,
    pub suspension_id: String,
    /// The image to resolve the Generation from, when the suspension did not keep one.
    pub image: Option<OciImage>,
}

/// Why a suspension or a resume did not happen.
#[derive(Debug)]
pub enum SuspensionFailure {
    /// The engine's typed failure; `Backend(Unsupported)` means the Instance keeps running.
    Managed(ManagedFailure),
    /// No Suspension of that identity is held here.
    Unknown,
    /// The Suspension expired and has been destroyed.
    Expired,
    /// The Suspension was taken on a host with another processor profile.
    OtherHost,
    /// Neither the request nor the Suspension names an image to resume from.
    NoImage,
    /// The Instance was suspended but the store refused the members; they are gone.
    Store(String),
}

impl From<ManagedFailure> for SuspensionFailure {
    fn from(failure: ManagedFailure) -> Self {
        Self::Managed(failure)
    }
}

/// What the runtime keeps beside a sealed Suspension, which the header does not carry.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Kept {
    image: Option<String>,
    shape: MachineShape,
}

/// The suspension store of one state root.
pub(crate) struct Suspensions {
    root: PathBuf,
}

impl Suspensions {
    pub(crate) fn at(state_root: &Path) -> Self {
        Self {
            root: state_root.join("suspensions"),
        }
    }

    fn store(&self) -> Store {
        Store::new(&self.root, self.root.join("master.key"))
    }

    fn kept_path(&self, id: SuspensionId) -> PathBuf {
        self.root.join("kept").join(format!("{}.json", id.hex()))
    }

    /// Creates the staging directory for a new Suspension.
    pub(crate) fn stage(&self) -> std::io::Result<(SuspensionId, PathBuf)> {
        let id = SuspensionId::new(random16()?)
            .map_err(|_| std::io::Error::other("zero suspension id"))?;
        let staging = self.root.join("staging").join(id.hex());
        create_private(&staging)?;
        Ok((id, staging))
    }

    /// Seals the members staged for `id` from what the suspended Instance's receipt states.
    pub(crate) fn seal(
        &self,
        id: SuspensionId,
        staging: &Path,
        request: &SuspendRequest,
        receipt: &ExecutionReceipt,
    ) -> Result<Suspended, SuspensionFailure> {
        let result = self.seal_members(id, staging, request, receipt);
        let _ignored = fs::remove_dir_all(staging);
        result
    }

    fn seal_members(
        &self,
        id: SuspensionId,
        staging: &Path,
        request: &SuspendRequest,
        receipt: &ExecutionReceipt,
    ) -> Result<Suspended, SuspensionFailure> {
        let store_failure = |what: &str| SuspensionFailure::Store(what.to_owned());
        let generation = receipt_generation(receipt).ok_or_else(|| store_failure("generation"))?;
        let shape = receipt.requested_shape();
        let created = now_unix();
        let retention = request.retention_secs.clamp(1, MAX_RETENTION_SECS);
        let header = Header {
            id,
            generation,
            host_profile: host_profile(),
            tenant: request.tenant.clone(),
            shape: Shape {
                vcpus: shape.vcpu_count(),
                memory_mib: u32::try_from(shape.memory_mib()).map_err(|_| store_failure("shape"))?,
                storage_mib: u32::try_from(shape.storage_mib())
                    .map_err(|_| store_failure("shape"))?,
            },
            created_unix: created,
            expires_unix: created + retention,
            members: [MemberRecord {
                bytes: 0,
                sha256: [0; 32],
            }; 3],
        };
        let kept = Kept {
            image: request.image.as_ref().map(|image| image.as_str().to_owned()),
            shape: shape.clone(),
        };
        let kept_path = self.kept_path(id);
        create_private(kept_path.parent().unwrap_or(&self.root))
            .map_err(|error| store_failure(&error.to_string()))?;
        fs::write(
            &kept_path,
            serde_json::to_vec(&kept).map_err(|error| store_failure(&error.to_string()))?,
        )
        .map_err(|error| store_failure(&error.to_string()))?;
        let sealed = self.store().seal(staging, header).map_err(|error| {
            let _ignored = fs::remove_file(&kept_path);
            store_failure(&describe(&error))
        })?;
        Ok(Suspended {
            suspension_id: id.hex(),
            expires_unix: sealed.expires_unix,
            sealed_bytes: sealed.members.iter().map(|member| member.bytes).sum(),
            receipt: receipt.clone(),
        })
    }

    /// Decrypts a Suspension into a fresh directory, ready for a resume launch.
    pub(crate) fn open(
        &self,
        request: &ResumeRequest,
    ) -> Result<Opened, SuspensionFailure> {
        let id = parse_id(&request.suspension_id).ok_or(SuspensionFailure::Unknown)?;
        let store = self.store();
        let header = store.header(id).map_err(|error| match error {
            StoreError::Missing => SuspensionFailure::Unknown,
            other => SuspensionFailure::Store(describe(&other)),
        })?;
        if header.host_profile != host_profile() {
            return Err(SuspensionFailure::OtherHost);
        }
        let kept: Kept = fs::read(self.kept_path(id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or(SuspensionFailure::Unknown)?;
        let image = match (&request.image, kept.image) {
            (Some(image), _) => image.clone(),
            (None, Some(reference)) => {
                OciImage::parse(reference).map_err(|_| SuspensionFailure::NoImage)?
            }
            (None, None) => return Err(SuspensionFailure::NoImage),
        };
        let directory = self
            .root
            .join("open")
            .join(format!("{}-{}", id.hex(), request.operation_id.as_str()));
        let _ignored = fs::remove_dir_all(&directory);
        create_private(&directory).map_err(|error| SuspensionFailure::Store(error.to_string()))?;
        let opened = store
            .open(id, now_unix(), &directory)
            .map_err(|error| match error {
                StoreError::Expired => SuspensionFailure::Expired,
                StoreError::Missing => SuspensionFailure::Unknown,
                other => SuspensionFailure::Store(describe(&other)),
            })
            .and_then(|header| {
                fs::write(
                    directory.join(RESUME_GENERATION_FILE),
                    header.generation.hex(),
                )
                .map_err(|error| SuspensionFailure::Store(error.to_string()))
            });
        if let Err(failure) = opened {
            let _ignored = fs::remove_dir_all(&directory);
            if matches!(failure, SuspensionFailure::Expired) {
                let _ignored = fs::remove_file(self.kept_path(id));
            }
            return Err(failure);
        }
        Ok(Opened {
            id,
            directory,
            image,
            shape: kept.shape,
        })
    }

    /// Destroys a Suspension, after a resume ran it or when its owner discards it.
    pub(crate) fn destroy(&self, suspension_id: &str) -> Result<(), SuspensionFailure> {
        let id = parse_id(suspension_id).ok_or(SuspensionFailure::Unknown)?;
        self.store()
            .destroy(id)
            .map_err(|error| SuspensionFailure::Store(describe(&error)))?;
        let _ignored = fs::remove_file(self.kept_path(id));
        Ok(())
    }
}

/// A Suspension decrypted for one resume.
pub(crate) struct Opened {
    pub(crate) id: SuspensionId,
    pub(crate) directory: PathBuf,
    pub(crate) image: OciImage,
    pub(crate) shape: MachineShape,
}

impl Opened {
    /// The launch that resumes this Suspension as `request`'s new Instance.
    pub(crate) fn launch(&self, request: &ResumeRequest) -> LaunchMachineRequest {
        LaunchMachineRequest::new(
            request.operation_id.clone(),
            request.instance_id.clone(),
            self.image.clone(),
            self.shape.clone(),
        )
        .with_resume(self.directory.clone())
    }
}

/// The engine request that reads one Instance into a staging directory.
pub(crate) fn engine_request(request: &SuspendRequest, staging: &Path) -> SuspendMachineRequest {
    SuspendMachineRequest::new(
        request.operation_id.clone(),
        request.instance_id.clone(),
        staging.to_path_buf(),
    )
}

fn receipt_generation(receipt: &ExecutionReceipt) -> Option<GenerationId> {
    let WorkloadEvidence::Resolved { identity } = receipt.workload() else {
        return None;
    };
    let hex = identity.generation_id()?.as_str().strip_prefix("sha256:")?;
    GenerationId::new(hex32(hex)?).ok()
}

fn parse_id(text: &str) -> Option<SuspensionId> {
    if text.len() != 32 {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    SuspensionId::new(bytes).ok()
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(bytes)
}

/// The processor this host restores on: a Suspension's vCPU state is only valid on the same one.
fn host_profile() -> HostProfileDigest {
    let mut hash = Sha256::new();
    hash.update(b"soma.suspension.host-profile.v1\0");
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    // The first processor's identity and features; every processor of one host agrees on them.
    for line in cpuinfo.lines().take_while(|line| !line.is_empty()) {
        let key = line.split(':').next().unwrap_or("").trim();
        if matches!(key, "vendor_id" | "cpu family" | "model" | "model name" | "flags") {
            hash.update(line.as_bytes());
            hash.update(b"\n");
        }
    }
    let digest: [u8; 32] = hash.finalize().into();
    // A SHA-256 digest is never all zero, so the identity is always valid.
    HostProfileDigest::new(digest).unwrap_or_else(|_| unreachable!("sha256 digest is nonzero"))
}

fn random16() -> std::io::Result<[u8; 16]> {
    let mut bytes = [0_u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn create_private(directory: &Path) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn describe(error: &StoreError) -> String {
    match error {
        StoreError::Io(error) => format!("io: {error}"),
        StoreError::Key(error) => format!("key: {error:?}"),
        StoreError::Seal(error) => format!("seal: {error:?}"),
        StoreError::Header(error) => format!("header: {error}"),
        StoreError::Expired => "expired".to_owned(),
        StoreError::Missing => "missing".to_owned(),
    }
}
