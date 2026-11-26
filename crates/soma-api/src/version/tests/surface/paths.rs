//! The path template a route serves and one concrete path for it.

use super::{IMPORT, INSTANCE, Route, SUSPENSION, VOLUME, VolumeOperation};
use crate::route::TransferRoute;

/// The path template of a route, with `{instance}`, `{port}`, `{volume}`, `{suspension}`, and
/// `{import}` for its values.
pub(super) fn template(route: &Route) -> String {
    match route {
        Route::Version => "/v1/version".to_owned(),
        Route::Prewarm(_) => "/v1/prewarm".to_owned(),
        Route::PrunePrepared => "/v1/prepared/prune".to_owned(),
        Route::Capacity => "/v1/capacity".to_owned(),
        Route::Autosize(..) => "/v1/sandboxes/{instance}/autosize".to_owned(),
        Route::CreateSandbox | Route::ListSandboxes => "/v1/sandboxes".to_owned(),
        Route::GetSandbox(_) | Route::DestroySandbox(_) => "/v1/sandboxes/{instance}".to_owned(),
        Route::PauseSandbox(_) => "/v1/sandboxes/{instance}/pause".to_owned(),
        Route::ResumeSandbox(_) => "/v1/sandboxes/{instance}/resume".to_owned(),
        Route::CheckpointSandbox(_) => "/v1/sandboxes/{instance}/checkpoints".to_owned(),
        Route::DiscardCheckpoint(_) => "/v1/sandboxes/{instance}/checkpoints/discard".to_owned(),
        Route::RestoreSandbox(_) => "/v1/sandboxes/{instance}/restore".to_owned(),
        Route::ForkSandbox(_) => "/v1/sandboxes/{instance}/fork".to_owned(),
        Route::StopSandbox(_) => "/v1/sandboxes/{instance}/stop".to_owned(),
        Route::SuspendSandbox(_) => "/v1/sandboxes/{instance}/suspend".to_owned(),
        Route::ResumeSuspension(_) => "/v1/suspensions/{suspension}/resume".to_owned(),
        Route::DestroySuspension(_) => "/v1/suspensions/{suspension}".to_owned(),
        Route::RunCommand(_) => "/v1/sandboxes/{instance}/commands".to_owned(),
        Route::Filesystem(_, operation) => {
            format!("/v1/sandboxes/{{instance}}/filesystem/{}", operation.name())
        }
        Route::Terminal(_, operation) => {
            format!("/v1/sandboxes/{{instance}}/terminal/{}", operation.name())
        }
        Route::Tunnel(_, _) => "/v1/sandboxes/{instance}/tunnel/{port}".to_owned(),
        Route::Resources(_, _) => "/v1/sandboxes/{instance}/resources".to_owned(),
        Route::Volume(VolumeOperation::Read, _) => "/v1/volumes/{volume}".to_owned(),
        Route::Volume(operation, _) => format!(
            "/v1/volumes/{}",
            operation.name().trim_start_matches("volume.")
        ),
        Route::CheckpointTransfer(TransferRoute::Export) => "/v1/checkpoints/export".to_owned(),
        Route::CheckpointTransfer(TransferRoute::Receive(_, artifact)) => {
            format!("/v1/checkpoints/imports/{{import}}/{}", artifact.name())
        }
        Route::CheckpointTransfer(TransferRoute::Commit(_)) => {
            "/v1/checkpoints/imports/{import}/commit".to_owned()
        }
        Route::CheckpointTransfer(TransferRoute::Abort(_)) => {
            "/v1/checkpoints/imports/{import}".to_owned()
        }
    }
}

pub(super) fn concrete(template: &str) -> String {
    template
        .replace("{instance}", INSTANCE)
        .replace("{port}", "8080")
        .replace("{volume}", VOLUME)
        .replace("{suspension}", SUSPENSION)
        .replace("{import}", IMPORT)
}
