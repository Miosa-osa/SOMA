//! Shared harness for the integration tests: an in-process pool over a fresh ledger.

#![allow(dead_code)]

use std::{path::Path, sync::Arc, time::Duration};

use soma_hostd::{
    AssignmentIntent, CpuClass, ExhaustedBehavior, GenerationId, HostProfileDigest, InstanceId,
    LaunchMaterialHandle, Limits, MemoryClass, MemoryShape, OperationId, OverlayIdentity, Pool,
    PoolKey, WorkloadClass,
    testing::{InProcessBroker, InProcessLauncher, ProcessTable},
};
use soma_netd::{EgressClass, NetworkIntent, ProfileDigest};
use soma_storage::{ClassName, TemplateDigest};
use tempfile::TempDir;

pub type TestPool = Pool<InProcessLauncher, InProcessBroker>;

pub struct Harness {
    pub dir: TempDir,
    pub table: Arc<ProcessTable>,
    pub pool: Arc<TestPool>,
}

/// A ledger directory on a memory-backed filesystem when one exists, so the fsync of every
/// record measures the ledger protocol rather than the development disk.
pub fn ledger_dir() -> TempDir {
    if let Some(override_dir) = std::env::var_os("SOMA_HOSTD_LEDGER_DIR") {
        return tempfile::Builder::new()
            .prefix("soma-hostd-")
            .tempdir_in(override_dir)
            .expect("tempdir in SOMA_HOSTD_LEDGER_DIR");
    }
    let shm = Path::new("/dev/shm");
    if shm.is_dir() {
        tempfile::Builder::new()
            .prefix("soma-hostd-")
            .tempdir_in(shm)
            .expect("tempdir in /dev/shm")
    } else {
        tempfile::Builder::new()
            .prefix("soma-hostd-")
            .tempdir()
            .expect("tempdir")
    }
}

pub fn key() -> PoolKey {
    PoolKey {
        host_profile: HostProfileDigest::new([1; 32]).expect("nonzero"),
        generation: GenerationId::new([2; 32]).expect("nonzero"),
        cpu: CpuClass {
            vcpus: 1,
            workload: WorkloadClass::ApiWaiting,
        },
        memory: MemoryShape {
            guest_bytes: 512 << 20,
            class: MemoryClass::Guaranteed,
        },
        overlay: OverlayIdentity {
            name: ClassName::new("small").expect("name"),
            version: 1,
            logical_bytes: 4 << 30,
            template: TemplateDigest::from_bytes([3; 32]),
        },
        network: ProfileDigest([4; 32]),
    }
}

pub fn limits(target: usize, max: usize) -> Limits {
    Limits {
        min: target.min(1),
        target,
        max,
        replenish_concurrency: 4,
        claim_deadline: Duration::from_secs(5),
        construction_deadline: Duration::from_secs(5),
        exhausted: ExhaustedBehavior::Reject,
        binding_limit: max.max(1) * 64,
    }
}

pub fn harness(limits: Limits) -> Harness {
    let dir = ledger_dir();
    let table = ProcessTable::new();
    let pool = open(dir.path(), &table, limits);
    Harness { dir, table, pool }
}

pub fn open(dir: &Path, table: &Arc<ProcessTable>, limits: Limits) -> Arc<TestPool> {
    Arc::new(
        Pool::open(
            key(),
            limits,
            InProcessLauncher::new(Arc::clone(table)),
            InProcessBroker::new(),
            dir,
        )
        .expect("pool"),
    )
}

pub fn op(n: u32) -> OperationId {
    let mut bytes = [0xa0; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    OperationId::new(bytes).expect("nonzero")
}

pub fn instance(n: u32) -> InstanceId {
    let mut bytes = [0xb0; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    InstanceId::new(bytes).expect("nonzero")
}

pub fn intent(n: u32) -> AssignmentIntent {
    AssignmentIntent {
        instance: instance(n),
        operation: op(n),
        vsock_cid: 3 + n,
        network: NetworkIntent::new(
            EgressClass::Denied,
            Vec::new(),
            Vec::new(),
            ProfileDigest([4; 32]),
        )
        .expect("intent"),
        deadline: Duration::from_secs(60),
        launch_material: LaunchMaterialHandle::new([9; 32]).expect("nonzero"),
    }
}
