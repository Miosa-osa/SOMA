use crate::{InstanceId, StateRevision, StateStore, StateStoreFailureKind};

use super::{
    Engine,
    machine_state::{DurableMachine, VersionedMachine},
    managed::ManagedFailure,
};

impl<B, S: StateStore> Engine<B, S> {
    pub(super) fn load_machine(
        &mut self,
        instance_id: &InstanceId,
    ) -> Result<Option<VersionedMachine>, ManagedFailure> {
        self.state
            .load(instance_id)
            .map_err(store_failure)?
            .map(|stored| DurableMachine::decode(&stored, instance_id).map_err(store_failure))
            .transpose()
    }

    pub(super) fn create_machine(
        &mut self,
        machine: &DurableMachine,
    ) -> Result<StateRevision, ManagedFailure> {
        let record = machine.encode().map_err(store_failure)?;
        self.state
            .create(&machine.instance_id, record)
            .map_err(store_failure)
    }

    pub(super) fn replace_machine(
        &mut self,
        revision: StateRevision,
        machine: &DurableMachine,
    ) -> Result<StateRevision, ManagedFailure> {
        let record = machine.encode().map_err(store_failure)?;
        self.state
            .compare_exchange(&machine.instance_id, revision, record)
            .map_err(store_failure)
    }
}

pub(super) fn store_failure(failure: crate::StateStoreFailure) -> ManagedFailure {
    ManagedFailure::StateStore(failure.kind())
}

pub(super) fn is_store_conflict(failure: &ManagedFailure) -> bool {
    matches!(
        failure,
        ManagedFailure::StateStore(StateStoreFailureKind::Conflict)
    )
}
