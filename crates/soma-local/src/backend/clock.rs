use std::{collections::HashMap, time::Instant};

use soma::OperationId;

const MAX_TRACKED_OPERATIONS: usize = 1_024;

pub(super) struct OperationClocks {
    starts: HashMap<OperationId, Instant>,
}

impl OperationClocks {
    pub(super) fn new() -> Self {
        Self {
            starts: HashMap::new(),
        }
    }

    pub(super) fn elapsed_ns(&mut self, operation_id: &OperationId) -> u64 {
        if !self.starts.contains_key(operation_id) && self.starts.len() >= MAX_TRACKED_OPERATIONS {
            self.starts.clear();
        }
        let started = self
            .starts
            .entry(operation_id.clone())
            .or_insert_with(Instant::now);
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}
