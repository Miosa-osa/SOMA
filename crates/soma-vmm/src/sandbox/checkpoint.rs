//! One checkpoint exchange with the thread that owns the running VM.

use std::{fs::File, time::Duration};

use soma_kvm::{
    MachineContract,
    snapshot::{Digest, manifest::CheckpointManifest},
    x86_64::{
        CheckpointError, CheckpointRequest, CheckpointTargets, SandboxMachine, capture_checkpoint,
    },
};

use super::{
    Session, SessionError,
    session::{Request, Response},
};

const KICK_GRACE: Duration = Duration::from_secs(2);

/// The launch identity and disk geometry selected by the owning broker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointSource {
    pub instance_id: [u8; 16],
    pub candidate_id: [u8; 32],
    pub root_digest: Digest,
    pub contract: MachineContract,
    pub has_overlay: bool,
    pub has_volume: bool,
}

/// Owns the descriptor capabilities transferred to the sandbox thread.
pub struct CheckpointJob {
    pub source: CheckpointSource,
    pub state: File,
    pub memory: File,
    pub overlay: Option<File>,
    pub volume: Option<File>,
}

/// The result distinguishes a staged-file failure from uncertain guest execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointJobError {
    CaptureRefused,
    SourceUnconfirmed,
}

impl CheckpointJob {
    /// Runs on the worker that has the original VM and authenticated session.
    ///
    /// # Errors
    ///
    /// Returns a refusal after the source resumed, or reports that the source state is uncertain.
    pub fn capture(
        &self,
        machine: &SandboxMachine,
    ) -> Result<CheckpointManifest, CheckpointJobError> {
        let CheckpointSource {
            instance_id,
            candidate_id,
            root_digest,
            contract,
            ..
        } = self.source;
        capture_checkpoint(
            machine,
            &CheckpointRequest {
                instance_id,
                candidate_id,
                root_digest,
                contract,
                grace: KICK_GRACE,
                targets: CheckpointTargets {
                    state: &self.state,
                    memory: &self.memory,
                    overlay: self.overlay.as_ref(),
                    volume: self.volume.as_ref(),
                },
            },
        )
        .map_err(|error| {
            eprintln!("soma-vmm: checkpoint: {error}");
            match error {
                CheckpointError::Capture(_) => CheckpointJobError::CaptureRefused,
                CheckpointError::Pause(_) | CheckpointError::Resume { .. } => {
                    CheckpointJobError::SourceUnconfirmed
                }
            }
        })
    }
}

impl Session {
    /// Runs a checkpoint while retaining the same live session.
    ///
    /// # Errors
    ///
    /// A capture refusal keeps the session usable. A lost response or an unconfirmed source
    /// state poisons it so a caller cannot mistake it for a known running machine.
    pub fn checkpoint(
        &mut self,
        job: CheckpointJob,
        deadline: Duration,
    ) -> Result<CheckpointManifest, CheckpointJobError> {
        if self.poisoned || self.settle().is_err() {
            return Err(CheckpointJobError::SourceUnconfirmed);
        }
        if self
            .requests
            .send(Request::Checkpoint(Box::new(job)))
            .is_err()
        {
            self.poison(SessionError::Gone);
            return Err(CheckpointJobError::SourceUnconfirmed);
        }
        match self.await_response(deadline) {
            Ok(Response::Checkpointed(Ok(manifest))) => Ok(*manifest),
            Ok(
                Response::Checkpointed(Err(CheckpointJobError::CaptureRefused))
                | Response::GuestPaused,
            ) => Err(CheckpointJobError::CaptureRefused),
            _ => {
                self.poison(SessionError::Gone);
                Err(CheckpointJobError::SourceUnconfirmed)
            }
        }
    }
}
