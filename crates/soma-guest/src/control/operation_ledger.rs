use std::collections::HashSet;

use crate::OperationId;

const MAX_SESSION_OPERATIONS: usize = 65_536;

pub(super) struct OperationLedger {
    seen: HashSet<OperationId>,
}

impl OperationLedger {
    pub(super) fn new(launch: OperationId) -> Self {
        Self {
            seen: HashSet::from([launch]),
        }
    }

    pub(super) fn reserve(&mut self, operation: OperationId) -> bool {
        if self.seen.len() == MAX_SESSION_OPERATIONS || !self.seen.insert(operation) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reuse() {
        let operation = operation(1);
        let mut ledger = OperationLedger::new(operation);

        assert!(!ledger.reserve(operation));
    }

    #[test]
    fn rejects_growth_beyond_the_session_bound() {
        let mut ledger = OperationLedger::new(operation(1));
        let limit = u128::try_from(MAX_SESSION_OPERATIONS).expect("operation bound fits u128");
        for value in 2..=limit {
            assert!(ledger.reserve(operation(value)));
        }

        assert!(!ledger.reserve(operation(limit + 1)));
    }

    fn operation(value: u128) -> OperationId {
        OperationId::new(value.to_be_bytes()).expect("nonzero operation")
    }
}
