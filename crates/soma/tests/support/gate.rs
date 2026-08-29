use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone)]
pub struct CallGate {
    state: Arc<(Mutex<GateState>, Condvar)>,
}

#[derive(Default)]
struct GateState {
    started: bool,
    released: bool,
}

#[allow(
    dead_code,
    reason = "shared concurrency fixture is used by selected integration tests"
)]
impl CallGate {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(GateState::default()), Condvar::new())),
        }
    }

    pub fn wait_until_started(&self) {
        let (lock, changed) = &*self.state;
        let state = lock.lock().expect("gate poisoned");
        drop(
            changed
                .wait_while(state, |state| !state.started)
                .expect("gate poisoned"),
        );
    }

    pub fn release(&self) {
        let (lock, changed) = &*self.state;
        lock.lock().expect("gate poisoned").released = true;
        changed.notify_all();
    }

    pub(super) fn block_backend(&self) {
        let (lock, changed) = &*self.state;
        let mut state = lock.lock().expect("gate poisoned");
        state.started = true;
        changed.notify_all();
        drop(
            changed
                .wait_while(state, |state| !state.released)
                .expect("gate poisoned"),
        );
    }
}
