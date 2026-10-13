//! The per-Instance durable data volume of ADR 0053 Step A.
//!
//! A volume is a distinct named object with its own device, its own explicit attach and detach
//! transactions, its own crash recovery, and its own deletion semantics, owned by the Host
//! Runtime rather than by the Instance, which is the shape ADR 0036 fixes.
//! This module owns the durable description of a volume, the write-ahead records its lifecycle
//! is built from, the rebuild that turns those records and the on-disk sidecars back into the
//! set of stopped volumes after a reboot, the offline repair receipt, the lifecycle seam the
//! daemon drives, and the placement rule that keeps those records off a volatile mount.
//! It owns no guest device logic and no host path outside the record root.

pub mod error;
pub mod ext4;
pub mod keyring;
pub mod placement;
pub mod rebuild;
pub mod record;
pub mod repair;
pub mod service;
pub mod sidecar;
pub mod store;

pub use error::VolumeError;
pub use ext4::{Ext4, outcome_from_exit};
pub use keyring::{HostKeyring, KeyError, SealedKey, VolumeKeys};
pub use placement::{
    DEFAULT_RECORD_DIR, DEFAULT_VOLUME_DIR, RECORD_DIR_ENV, on_volatile_mount, record_dir,
    refuse_volatile,
};
pub use rebuild::{RebuildReport, StoppedVolume, rebuild};
pub use record::{
    VOLUME_RECORD_LEN, VOLUME_RECORD_MAGIC, VOLUME_RECORD_VERSION, VolumeRecord, VolumeRecordKind,
};
pub use repair::{
    CreateError, FORMAT_TOOL, REPAIR_TOOL, RepairOutcome, RepairReceipt, VolumeRepair,
};
pub use service::{
    AttachOutcome, AttachRequest, DetachOutcome, DetachRequest, VOLUME_DEVICE, VolumeStore,
};
pub use sidecar::{
    MAX_TENANT_BYTES, PERSISTENCE_PATHS, SIDECAR_LEN, SIDECAR_MAGIC, SIDECAR_VERSION, SidecarError,
    VolumeSidecar, VolumeState,
};
pub use store::{CleanRepair, DirectoryVolumes, SIDECAR_FILE, VOLUME_FILE};
