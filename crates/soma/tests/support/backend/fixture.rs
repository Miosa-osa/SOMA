use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::AtomicUsize},
};

use super::{CallGate, Mode, TestBackend};

#[allow(
    dead_code,
    reason = "shared gated backend variants are used by selected integration tests"
)]
impl TestBackend {
    pub fn new(mode: Mode) -> (Self, Arc<Mutex<Vec<&'static str>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                mode,
                calls: Arc::clone(&calls),
                execute_gate: None,
                cleanup_gate: None,
                files: Arc::new(Mutex::new(BTreeMap::new())),
                terminal: Arc::new(Mutex::new(None)),
                executions: Arc::new(AtomicUsize::new(0)),
            },
            calls,
        )
    }

    pub fn with_execute_gate(mut self) -> (Self, CallGate) {
        let gate = CallGate::new();
        self.execute_gate = Some(gate.clone());
        (self, gate)
    }

    pub fn with_cleanup_gate(mut self) -> (Self, CallGate) {
        let gate = CallGate::new();
        self.cleanup_gate = Some(gate.clone());
        (self, gate)
    }

    pub(super) fn record(&self, name: &'static str) {
        self.calls.lock().expect("call log poisoned").push(name);
    }

    pub(super) fn count(&self, name: &str) -> usize {
        let calls = self.calls.lock().expect("call log poisoned");
        calls.iter().filter(|c| **c == name).count()
    }
}
