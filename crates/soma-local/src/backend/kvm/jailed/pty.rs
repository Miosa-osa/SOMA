//! One bounded terminal operation against a jailed machine.

use soma::{BackendFailureKind, PtyAnswer, PtyOperation};
use soma_vmm::OperationId;
use soma_vmm::control::{PtyRequest, Request};

use super::{Jailed, outcome};
use crate::backend::kvm::identity::fresh16;

impl Jailed {
    pub(in crate::backend::kvm) fn pty(
        &mut self,
        operation: PtyOperation,
    ) -> Result<PtyAnswer, BackendFailureKind> {
        if self.poisoned {
            return Err(BackendFailureKind::Unavailable);
        }
        let operation_id =
            OperationId::new(fresh16()).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        let request = Request::Pty(PtyRequest::new(operation_id, self.instance, operation));
        match self.control.ask(&request, soma_vmm::sandbox::PTY_CEILING) {
            Ok(answer) => outcome::pty(answer).map_err(|kind| self.poison(kind)),
            Err(kind) => Err(self.poison(kind)),
        }
    }
}
