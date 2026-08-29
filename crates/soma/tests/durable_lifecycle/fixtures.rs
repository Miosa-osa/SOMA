use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use soma::{
    DirectCommand, Engine, ExecuteMachineRequest, ExecutionLimits, InstanceId,
    LaunchMachineRequest, MachineShape, MemoryStateStore, OciImage, OperationId, StateRecord,
    StateRevision, StateStore, StateStoreFailure, StateStoreFailureKind, StoredState,
};

use crate::support::{Mode, TestBackend};

#[derive(Clone)]
pub(super) struct FailingCasStore {
    inner: MemoryStateStore,
    failures: Arc<BTreeSet<usize>>,
    calls: Arc<AtomicUsize>,
}

impl FailingCasStore {
    pub(super) fn new<const N: usize>(failures: [usize; N]) -> Self {
        Self {
            inner: MemoryStateStore::new(),
            failures: Arc::new(failures.into_iter().collect()),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl StateStore for FailingCasStore {
    fn create(
        &mut self,
        instance_id: &InstanceId,
        record: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        self.inner.create(instance_id, record)
    }

    fn load(&mut self, instance_id: &InstanceId) -> Result<Option<StoredState>, StateStoreFailure> {
        self.inner.load(instance_id)
    }

    fn compare_exchange(
        &mut self,
        instance_id: &InstanceId,
        expected: StateRevision,
        replacement: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.failures.contains(&call) {
            return Err(StateStoreFailure::new(StateStoreFailureKind::Unavailable));
        }
        self.inner
            .compare_exchange(instance_id, expected, replacement)
    }
}

pub(super) struct StaticStore {
    pub(super) record: StateRecord,
}

impl StateStore for StaticStore {
    fn create(
        &mut self,
        _instance_id: &InstanceId,
        _record: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        Err(StateStoreFailure::new(StateStoreFailureKind::Conflict))
    }

    fn load(
        &mut self,
        _instance_id: &InstanceId,
    ) -> Result<Option<StoredState>, StateStoreFailure> {
        Ok(Some(StoredState::new(
            StateRevision::INITIAL,
            self.record.clone(),
        )))
    }

    fn compare_exchange(
        &mut self,
        _instance_id: &InstanceId,
        _expected: StateRevision,
        _replacement: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        Err(StateStoreFailure::new(StateStoreFailureKind::Conflict))
    }
}

pub(super) fn launch_request(
    operation_id: OperationId,
    instance_id: InstanceId,
) -> LaunchMachineRequest {
    LaunchMachineRequest::new(
        operation_id,
        instance_id,
        OciImage::parse("node:22").expect("image"),
        MachineShape::new(1, 1_024, 8_192).expect("shape"),
    )
}

pub(super) fn execute_request(
    operation_id: OperationId,
    instance_id: InstanceId,
) -> ExecuteMachineRequest {
    ExecuteMachineRequest::new(
        operation_id,
        instance_id,
        DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
        ExecutionLimits::new(30_000, 1_048_576).expect("limits"),
    )
}

pub(super) fn operation(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}

pub(super) fn instance() -> InstanceId {
    InstanceId::new("22222222222222222222222222222222").expect("instance")
}

pub(super) fn valid_active_record() -> StateRecord {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store.clone());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch fixture");
    store
        .load(&instance())
        .expect("load fixture")
        .expect("fixture record")
        .record()
        .clone()
}
