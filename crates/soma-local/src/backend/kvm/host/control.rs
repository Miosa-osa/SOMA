//! Workspace, pause, resume, checkpoint, resize and egress requests to a hosted worker.

use super::{
    Answer, BackendFailureKind, CONTROL_CEILING, Call, HostFailure, InstanceId, Path, ask,
};

/// Reports whether a host process is serving this Instance right now.
///
/// It connects and does nothing else. What that proves is exactly one thing and it is worth
/// naming: a process has the Instance's socket bound and accepted a connection on it. It is not
/// a claim that the guest inside is healthy, which is what an inspection by exact identity
/// answers; it is the difference between a durable record whose host is running and one whose
/// host is gone.
///
/// A socket nothing answers on is removed by the connect itself, so a host that died leaves no
/// name behind for a second listing to report.
/// Sends an attachment or detachment for the owning process to serialize.
pub(crate) fn workspace(
    directory: &Path,
    instance: &InstanceId,
    request: &soma_hostd::Request,
) -> Result<soma_hostd::Reply, BackendFailureKind> {
    let request = Call::Workspace {
        instance_id: instance.clone(),
        request: request.encode(),
    };
    match ask(directory, instance, &request, CONTROL_CEILING) {
        Ok(Answer::WorkspaceVolume(reply)) => {
            soma_hostd::Reply::decode(&reply).map_err(|_| BackendFailureKind::GuestFailure)
        }
        Ok(Answer::Refused(kind)) => Err(kind.into()),
        Err(HostFailure::Refused(kind)) => Err(kind),
        Ok(_) => Err(BackendFailureKind::GuestFailure),
        Err(HostFailure::Absent) => Err(BackendFailureKind::Unavailable),
    }
}

/// Asks the owning host to confirm that guest execution is paused.
pub(crate) fn pause(directory: &Path, instance: &InstanceId) -> Result<(), crate::PauseFailure> {
    let call = Call::Pause {
        instance_id: instance.clone(),
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::Paused) => Ok(()),
        Ok(Answer::PauseRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::PauseFailure::NotFound),
        _ => Err(crate::PauseFailure::Unavailable),
    }
}

/// Asks the owning host to confirm that guest execution is resumed.
pub(crate) fn resume(directory: &Path, instance: &InstanceId) -> Result<(), crate::ResumeFailure> {
    let call = Call::Resume {
        instance_id: instance.clone(),
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::Resumed) => Ok(()),
        Ok(Answer::ResumeRefused(failure)) => {
            eprintln!("soma: owning host refused resident resume: {failure:?}");
            Err(failure)
        }
        Err(HostFailure::Absent) => Err(crate::ResumeFailure::NotFound),
        Err(HostFailure::Refused(kind)) => {
            eprintln!("soma: resident resume host exchange failed: {kind:?}");
            Err(crate::ResumeFailure::Unavailable)
        }
        Ok(_) => {
            eprintln!("soma: owning host returned an unexpected resume receipt");
            Err(crate::ResumeFailure::Unavailable)
        }
    }
}

/// Asks the owning host to durably checkpoint its running Instance.
pub(crate) fn checkpoint(
    directory: &Path,
    instance: &InstanceId,
    operation: &soma::OperationId,
) -> Result<crate::CheckpointReceipt, crate::CheckpointFailure> {
    let call = Call::Checkpoint {
        instance_id: instance.clone(),
        operation_id: operation.clone(),
    };
    let ceiling = super::super::checkpoint::CHECKPOINT_CEILING + CONTROL_CEILING;
    match ask(directory, instance, &call, ceiling) {
        Ok(Answer::Checkpointed(receipt))
            if &receipt.instance_id == instance && &receipt.checkpoint_id == operation =>
        {
            Ok(receipt)
        }
        Ok(Answer::CheckpointRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::CheckpointFailure::NotFound),
        _ => Err(crate::CheckpointFailure::Unconfirmed),
    }
}

/// Asks the host that holds `instance` to change its CPU and memory live.
pub(crate) fn resize(
    directory: &Path,
    instance: &InstanceId,
    request: crate::ResizeRequest,
) -> Result<crate::ResizeOutcome, crate::ResizeFailure> {
    let call = Call::Resize {
        instance_id: instance.clone(),
        request,
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::Resized(outcome)) => Ok(outcome),
        Ok(Answer::ResizeRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::ResizeFailure::NotFound),
        _ => Err(crate::ResizeFailure::Unavailable),
    }
}

/// Asks the host that holds `instance` to open or close its egress.
///
/// Anything that is not the host's own outcome is a failure the caller must read as "the egress
/// may not be what I asked for", including a host that does not know the call.
pub(crate) fn set_egress(
    directory: &Path,
    instance: &InstanceId,
    request: crate::EgressSetRequest,
) -> Result<crate::EgressSetOutcome, crate::EgressSetFailure> {
    let call = Call::SetEgress {
        instance_id: instance.clone(),
        request,
    };
    match ask(directory, instance, &call, CONTROL_CEILING) {
        Ok(Answer::EgressSet(outcome)) => Ok(outcome),
        Ok(Answer::EgressRefused(failure)) => Err(failure),
        Err(HostFailure::Absent) => Err(crate::EgressSetFailure::NotFound),
        _ => Err(crate::EgressSetFailure::Unavailable),
    }
}
