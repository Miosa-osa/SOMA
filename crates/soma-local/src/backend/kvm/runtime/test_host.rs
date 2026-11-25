//! A real Host daemon on a temporary socket, served by the in-process launcher.

use super::*;

/// One Host daemon over a prepared in-process pool, serving a socket for these tests.
pub(super) struct Host {
    socket: PathBuf,
    /// The directory holding the ledger and the socket, removed when the test ends.
    _directory: tempfile::TempDir,
}

impl Host {
    pub(super) fn serve() -> Self {
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
        // The daemon serves the volume seam too (ADR 0053 Step A); this host has no volume
        // to serve, so the store is the reference one on its own directory.
        let volumes = DirectoryVolumes::open_uncertified(
            &directory.path().join("volumes"),
            &directory.path().join("volume-records"),
            Box::new(CleanRepair),
            true,
        )
        .expect("volume store");
        let runtime = Arc::new(Runtime::new(pool));
        let served = socket.clone();
        thread::spawn(move || {
            let _ignored = daemon::serve(&runtime, &Arc::new(volumes), &served);
        });
        await_socket(&socket);
        Self {
            socket,
            _directory: directory,
        }
    }

    /// The socket the daemon serves.
    pub(super) fn socket(&self) -> &Path {
        &self.socket
    }

    /// One fresh client connection, exactly as a separate process would open one.
    pub(super) fn connect(&self) -> Ownership {
        Ownership::resolve(Some(OsStr::new(&self.socket))).expect("the daemon is serving")
    }
}

pub(super) fn await_socket(socket: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && !socket.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(socket.exists(), "the daemon never bound its socket");
}

pub(super) fn limits() -> Limits {
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

pub(super) fn key() -> PoolKey {
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

pub(super) fn shape() -> ValidShape {
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
pub(super) fn profile() -> HostProfile {
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
