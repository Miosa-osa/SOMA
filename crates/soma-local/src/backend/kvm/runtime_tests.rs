//! The ownership seam, proved against a real Host daemon on a real socket.
//!
//! The daemon here is served by the in-process development launcher, so these tests prove
//! exactly what this change claims and nothing more: the Instance identity is owned by the
//! Host and is therefore addressable and terminable from a client that did not create it. The
//! guest session is not part of that claim, because it is still resident in the process that
//! launched the machine.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use soma::{InstanceId, OperationId};
use soma_hostd::{
    Admission, CpuClass, CpuInventory, ExhaustedBehavior, GenerationId, HostProfile, Limits,
    MachineShape, MeasuredOverhead, MemoryClass, MemoryInventory, MemoryShape, NetworkInventory,
    OperatorLimits, OvercommitPolicy, OverlayIdentity, Pool, PoolAdmission, PoolKey,
    ProcessInventory, Runtime, SingleNode, StorageInventory, ValidShape, WorkloadClass,
    client::ClientError,
    daemon,
    testing::{InProcessBroker, InProcessLauncher, ProcessTable},
};
use soma_netd::ProfileDigest;
use soma_storage::{ClassName, TemplateDigest};

use super::Ownership;

/// How long a test waits for the daemon to bind its socket.
const PATIENCE: Duration = Duration::from_secs(5);

fn instance(tag: char) -> InstanceId {
    InstanceId::new(std::iter::repeat_n(tag, 32).collect::<String>()).expect("instance")
}

fn operation(tag: char) -> OperationId {
    OperationId::new(std::iter::repeat_n(tag, 32).collect::<String>()).expect("operation")
}

#[test]
fn no_configured_locator_keeps_the_in_process_lifecycle() {
    let ownership = Ownership::resolve(None).expect("an unconfigured Host resolves");
    assert!(
        matches!(ownership, Ownership::InProcess),
        "a Host with no Runtime owns its Instances in this process, as it always has"
    );
    let instance = instance('1');
    ownership
        .register(&instance, &operation('2'), 3)
        .expect("registration is nothing to do without a Host Runtime");
    assert!(
        !ownership.is_live(&instance),
        "nothing outside this process can be asked about the Instance"
    );
    assert!(
        ownership.release(&instance).expect("release"),
        "ownership no Host holds is ended by definition"
    );
}

#[test]
fn a_configured_host_runtime_that_is_not_serving_is_refused_rather_than_ignored() {
    let absent = std::env::temp_dir().join("soma-local-no-such-hostd.sock");
    let error = Ownership::resolve(Some(absent.as_os_str()))
        .err()
        .expect("a configured Runtime that nothing serves is a failure");
    assert!(
        matches!(error, ClientError::Connect(_)),
        "the refusal names the connection, so an operator is not told the request was rejected"
    );
}

#[test]
fn a_registered_instance_is_addressable_from_a_second_client() {
    let host = Host::serve();
    let instance = instance('3');

    let launcher = host.connect();
    launcher
        .register(&instance, &operation('4'), 7)
        .expect("the Host accepts the registration");
    // Dropping the client that registered the Instance closes its connection and nothing else,
    // which is the whole difference from the in-process handle this replaces.
    drop(launcher);

    let observer = host.connect();
    assert!(
        observer.is_live(&instance),
        "a client that never launched the Instance still addresses it by identity"
    );
    assert!(
        !observer.is_live(&self::instance('5')),
        "an identity the Host never launched is not reported live"
    );
}

#[test]
fn a_release_from_a_second_client_is_terminal() {
    let host = Host::serve();
    let instance = instance('6');

    let launcher = host.connect();
    launcher
        .register(&instance, &operation('7'), 8)
        .expect("the Host accepts the registration");
    drop(launcher);

    let destroyer = host.connect();
    assert!(
        destroyer.release(&instance).expect("release"),
        "a client that never launched the Instance may still end it"
    );
    assert!(
        destroyer.release(&instance).expect("repeat"),
        "the repeat is answered from the durable record rather than refused"
    );
    assert!(
        !destroyer.is_live(&instance),
        "the released Instance is no longer live"
    );
}

/// One Host daemon over a prepared in-process pool, serving a socket for these tests.
struct Host {
    socket: PathBuf,
    /// The directory holding the ledger and the socket, removed when the test ends.
    _directory: tempfile::TempDir,
}

impl Host {
    fn serve() -> Self {
        let directory = tempfile::tempdir().expect("directory");
        let socket = directory.path().join("hostd.sock");
        let pool = Arc::new(
            Pool::open(
                key(),
                limits(),
                InProcessLauncher::new(ProcessTable::new()),
                InProcessBroker::new(),
                PoolAdmission::new(
                    Arc::new(Admission::new(
                        profile().validate().expect("profile"),
                        SingleNode,
                    )),
                    shape(),
                ),
                &directory.path().join("ledger"),
            )
            .expect("pool"),
        );
        pool.replenish_blocking().expect("replenish");
        let runtime = Arc::new(Runtime::new(pool));
        let served = socket.clone();
        thread::spawn(move || {
            let _ignored = daemon::serve(&runtime, &served);
        });
        await_socket(&socket);
        Self {
            socket,
            _directory: directory,
        }
    }

    /// One fresh client connection, exactly as a separate process would open one.
    fn connect(&self) -> Ownership {
        Ownership::resolve(Some(OsStr::new(&self.socket))).expect("the daemon is serving")
    }
}

fn await_socket(socket: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && !socket.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(socket.exists(), "the daemon never bound its socket");
}

fn limits() -> Limits {
    Limits {
        min: 1,
        target: 2,
        max: 4,
        replenish_concurrency: 2,
        claim_deadline: Duration::from_secs(5),
        construction_deadline: Duration::from_secs(5),
        exhausted: ExhaustedBehavior::Reject,
        binding_limit: 64,
    }
}

fn key() -> PoolKey {
    PoolKey {
        host_profile: profile().digest(),
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

fn shape() -> ValidShape {
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

/// A host large enough that the pool bounds, not capacity, decide what these tests observe.
fn profile() -> HostProfile {
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
