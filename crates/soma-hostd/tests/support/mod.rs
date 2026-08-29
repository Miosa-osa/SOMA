//! Shared harness for the integration tests: an in-process pool over a fresh ledger.

#![allow(dead_code)]

use std::{path::Path, sync::Arc, time::Duration};

use soma_hostd::{
    Admission, AssignmentIntent, CpuClass, CpuInventory, ExhaustedBehavior, GenerationId,
    HostProfile, InstanceId, LaunchMaterialHandle, Limits, MachineShape, MeasuredOverhead,
    MemoryClass, MemoryInventory, MemoryShape, NetworkInventory, OperationId, OperatorLimits,
    OvercommitPolicy, OverlayIdentity, Pool, PoolAdmission, PoolKey, ProcessInventory, Request,
    SingleNode, StorageInventory, ValidShape, WorkloadClass,
    testing::{InProcessBroker, InProcessLauncher, ProcessTable},
};
use soma_netd::{EgressClass, NetworkIntent, ProfileDigest};
use soma_storage::{ClassName, TemplateDigest};
use tempfile::TempDir;

mod broker;

pub use broker::SharedBroker;

pub type TestPool = Pool<InProcessLauncher, InProcessBroker>;

/// A pool over the host broker every pool of a test shares.
pub type SharedPool = Pool<InProcessLauncher, SharedBroker>;

pub struct Harness {
    pub dir: TempDir,
    pub table: Arc<ProcessTable>,
    pub pool: Arc<TestPool>,
    pub admission: Arc<Admission>,
}

/// A host large enough that the pool bounds, not capacity, gate the policy tests.
pub fn host_profile() -> HostProfile {
    HostProfile {
        cpu: CpuInventory {
            hardware_threads: 100_000,
            reserved_threads: 1,
            numa_nodes: 1,
        },
        memory: MemoryInventory {
            total_bytes: 1 << 46,
            reserved_bytes: 1 << 30,
            overhead: MeasuredOverhead::ATLAS_PLACEHOLDER,
            elastic_budget_bytes: 0,
        },
        storage: StorageInventory {
            private_budget_bytes: u64::MAX / 2,
        },
        network: NetworkInventory { units: 100_000 },
        process: ProcessInventory {
            processes: 100_000,
            descriptors: 4_000_000,
        },
        limits: OperatorLimits {
            resident_instances: 100_000,
            concurrent_launches: 100_000,
            runnable_vcpus: 1_000_000,
            dirty_memory_bytes: u64::MAX / 2,
            cleanup_slots: 64,
        },
        overcommit: OvercommitPolicy::STRICT,
    }
}

/// The Machine shape every worker of the test pool is prepared for.
pub fn shape() -> ValidShape {
    MachineShape {
        vcpus: 1,
        guest_memory_bytes: 512 << 20,
        memory_class: MemoryClass::Guaranteed,
        private_storage_bytes: 4 << 30,
        workload: WorkloadClass::ApiWaiting,
        network_units: 1,
        descriptors: 16,
    }
    .validate()
    .expect("shape")
}

pub fn admission() -> Arc<Admission> {
    Arc::new(Admission::new(
        host_profile().validate().expect("profile"),
        SingleNode,
    ))
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
        host_profile: host_profile().digest(),
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
            template_digest: TemplateDigest::from_bytes([3; 32]),
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
    let admission = admission();
    let pool = open_with(dir.path(), &table, limits, &admission);
    Harness {
        dir,
        table,
        pool,
        admission,
    }
}

pub fn open(dir: &Path, table: &Arc<ProcessTable>, limits: Limits) -> Arc<TestPool> {
    open_with(dir, table, limits, &admission())
}

pub fn open_with(
    dir: &Path,
    table: &Arc<ProcessTable>,
    limits: Limits,
    admission: &Arc<Admission>,
) -> Arc<TestPool> {
    Arc::new(
        Pool::open(
            key(),
            limits,
            InProcessLauncher::new(Arc::clone(table)),
            InProcessBroker::new(),
            PoolAdmission::new(Arc::clone(admission), shape()),
            dir,
        )
        .expect("pool"),
    )
}

/// Opens a pool over one shared host broker, so a simulated restart keeps its leases.
pub fn open_shared(
    dir: &Path,
    table: &Arc<ProcessTable>,
    limits: Limits,
    admission: &Arc<Admission>,
    broker: &Arc<InProcessBroker>,
) -> Arc<SharedPool> {
    Arc::new(
        Pool::open(
            key(),
            limits,
            InProcessLauncher::new(Arc::clone(table)),
            SharedBroker::new(broker),
            PoolAdmission::new(Arc::clone(admission), shape()),
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

/// The daemon request that produces exactly `intent(n)`.
pub fn claim_request(n: u32) -> Request {
    let intent = intent(n);
    Request::Claim {
        operation: intent.operation,
        instance: intent.instance,
        vsock_cid: intent.vsock_cid,
        deadline_nanos: intent.deadline_nanos(),
        launch_material: intent.launch_material,
        intent: intent.network,
    }
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
