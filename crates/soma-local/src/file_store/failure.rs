use soma::{StateStoreFailure, StateStoreFailureKind};

pub(super) const fn invalid_record() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::InvalidRecord)
}

pub(super) const fn conflict() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Conflict)
}

pub(super) const fn unavailable() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Unavailable)
}

pub(super) const fn corrupt() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Corrupt)
}

pub(super) const fn capacity_exceeded() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::CapacityExceeded)
}
