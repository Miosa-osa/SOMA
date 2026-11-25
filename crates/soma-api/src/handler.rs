mod capacity;
pub(crate) mod checkpoint;
mod create;
mod pause;
mod prepared;
pub(crate) mod prewarm;
pub(crate) mod scope;
mod support;
pub(crate) mod suspension;
mod volume;
mod volumes;
use create::{create, lifecycle};
use support::{control_body, optional_body, parse, refuse, success};
use volume::binding as volume_binding;

use soma::{ExecutionReceipt, MachineState};

use crate::{
    capability::MissingCapability,
    envelope::{ApiError, Envelope, failure_body_with_result, render},
    facade::SandboxFacade,
    failure::managed_error,
    filesystem::{FilesystemBody, FilesystemReport},
    http::{request::Request, response::Response},
    report::{
        CommandReport, InspectionReport, OutputBytes, SandboxReport, VolumeReady, command_status,
    },
    resources::{ResourcesBody, ResourcesReport, nothing_applied, remember, replayed},
    route::{FilesystemOperation, Route, TerminalOperation, resolve},
    tenant::{TENANT_HEADER, identify},
    terminal::{TerminalBody, TerminalReport},
    volumes::VOLUME_DEVICE,
    wire::{CreateSandboxBody, RunCommandBody},
};

pub use prewarm::handle_prewarm;

/// Serves one request against the sandbox facade.
///
/// Identity is established before the route is resolved. That ordering is deliberate: an
/// unidentified caller learns only that it is unidentified, and cannot use the difference between
/// a 404 and a 405 to map the service's surface.
pub fn handle<F: SandboxFacade + ?Sized>(facade: &mut F, request: &Request) -> Response {
    match identify(request.header(TENANT_HEADER))
        .and_then(|tenant| resolve(request).map(|route| (tenant, route)))
    {
        Ok((tenant, route)) => {
            let operation = route.operation();
            dispatch(facade, route, &request.body, &tenant)
                .unwrap_or_else(|error| refuse(operation, error))
        }
        Err(error) => refuse("request", error),
    }
}

fn dispatch<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    route: Route,
    body: &[u8],
    tenant: &crate::tenant::TenantId,
) -> Result<Response, ApiError> {
    let operation = route.operation();
    scope::admit(facade, &route, tenant)?;
    match route {
        Route::CreateSandbox => create(facade, body, tenant),
        Route::ListSandboxes => scope::list(facade, operation, tenant),
        Route::Filesystem(instance_id, file_operation) => {
            filesystem(facade, instance_id, file_operation, body)
        }
        Route::Terminal(instance_id, terminal_operation) => {
            terminal(facade, instance_id, terminal_operation, body)
        }
        Route::GetSandbox(instance_id) => {
            let request = control_body(body)?.into_inspect(instance_id)?;
            let snapshot = facade.inspect(request).map_err(|f| managed_error(&f))?;
            Ok(success(
                200,
                operation,
                &InspectionReport {
                    instance_id: snapshot.instance_id,
                    state: snapshot.state,
                    backend: snapshot.backend,
                    reason: (snapshot.state == MachineState::Stopped).then_some("machine_lost"),
                },
                Some(&snapshot.receipt),
            ))
        }
        Route::PauseSandbox(instance_id) => {
            pause::serve(facade, instance_id, body, pause::Transition::Pause)
        }
        Route::ResumeSandbox(instance_id) => {
            pause::serve(facade, instance_id, body, pause::Transition::Resume)
        }
        Route::CheckpointSandbox(instance_id) => checkpoint::serve(facade, &instance_id, body),
        Route::DiscardCheckpoint(instance_id) => checkpoint::discard(facade, &instance_id, body),
        Route::RestoreSandbox(instance_id) => checkpoint::restore(facade, instance_id, body),
        Route::ForkSandbox(source) => checkpoint::fork(facade, &source, body, tenant),
        Route::StopSandbox(instance_id) => {
            let request = control_body(body)?.into_stop(instance_id.clone())?;
            let stop = request.operation_id().clone();
            let outcome = facade.stop(request).map_err(|f| managed_error(&f))?;
            // The stop released the volume the sandbox was running with, and its answer says so:
            // the state a stopped sandbox's data is in is part of what a stop reports.
            let volume = volume::detach_for_stop(facade, &instance_id, &stop)?;
            Ok(success(
                200,
                operation,
                &SandboxReport {
                    instance_id: outcome.instance_id,
                    state: "stopped",
                    volume,
                    fsck: None,
                },
                Some(&outcome.receipt),
            ))
        }
        Route::DestroySandbox(instance_id) => {
            let request = control_body(body)?.into_destroy(instance_id)?;
            let outcome = facade.destroy(request).map_err(|f| managed_error(&f))?;
            Ok(lifecycle(
                operation,
                "destroyed",
                &outcome.instance_id,
                &outcome.receipt,
            ))
        }
        // A tunnel is a connection, not a request and a response: the server hands it to the
        // tunnel pump before a handler runs, so reaching this arm means the caller used a
        // framing that cannot carry one.
        Route::Tunnel(..) => Err(ApiError::new(
            426,
            "upgrade_required",
            "a tunnel needs `Connection: Upgrade` and `Upgrade: soma-tunnel` over HTTP/1.1",
            false,
        )),
        Route::Resources(instance_id, change) => resources(facade, &instance_id, change, body),
        Route::Prewarm(_) | Route::Version => Err(ApiError::not_found()),
        Route::SuspendSandbox(id) => suspension::suspend(facade, &id, tenant.as_str(), body),
        Route::ResumeSuspension(id) => suspension::resume(facade, &id, body),
        Route::DestroySuspension(id) => suspension::destroy(facade, &id),
        Route::PrunePrepared => prepared::prune(facade, operation, body),
        Route::Capacity => capacity::report(facade, operation),
        Route::Autosize(id, change) => crate::autosize::serve(facade, &id, change, body),
        Route::RunCommand(instance_id) => {
            let request = parse::<RunCommandBody>(body)?.into_facade(instance_id)?;
            let outcome = facade.execute(request).map_err(|f| managed_error(&f))?;
            Ok(success(
                200,
                operation,
                &CommandReport {
                    instance_id: outcome.instance_id,
                    execution: command_status(outcome.status)?,
                    stdout: OutputBytes::new(&outcome.stdout),
                    stderr: OutputBytes::new(&outcome.stderr),
                },
                Some(&outcome.receipt),
            ))
        }
        Route::Volume(volume_operation, path_volume) => {
            volumes::serve(facade, volume_operation, path_volume, body, tenant)
        }
    }
}

/// Reads or changes a running sandbox's CPU and memory.
///
/// A read, and a change that left a dimension unchanged, answer `200`. A change that applied
/// nothing and was refused (an old Generation, a failed attempt) is a `409` carrying the same
/// report, so the reason is in the place a success would have put the numbers.
fn resources<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance_id: &soma::InstanceId,
    change: bool,
    body: &[u8],
) -> Result<Response, ApiError> {
    let operation = if change {
        "sandbox.resize"
    } else {
        "sandbox.resources"
    };
    let (request, operation_id) = if change {
        let parsed = parse::<ResourcesBody>(body)?;
        let operation_id = parsed.operation()?.map(str::to_owned);
        (parsed.into_request()?, operation_id)
    } else {
        (soma_local::ResizeRequest::default(), None)
    };
    if let Some(id) = &operation_id
        && let Some((status, stored)) = replayed(instance_id, id)
    {
        return Ok(Response::new(status, stored));
    }
    let outcome = facade
        .resize(instance_id, request)
        .map_err(|failure| match failure {
            soma_local::ResizeFailure::NotFound => ApiError::new(
                404,
                "machine_not_found",
                "sandbox is not running on this host",
                false,
            ),
            soma_local::ResizeFailure::Unavailable => ApiError::new(
                503,
                "resize_unavailable",
                "the sandbox could not be reached to resize it",
                true,
            ),
        })?;
    let report = ResourcesReport::new(instance_id, operation_id.as_deref(), &outcome);
    let response = if change && nothing_applied(&outcome) {
        let error = ApiError::new(
            409,
            "resize_not_applied",
            "nothing was applied; the report says why for each dimension",
            false,
        );
        Response::new(409, failure_body_with_result(operation, error, &report))
    } else {
        success(200, operation, &report, None)
    };
    if let Some(id) = &operation_id {
        remember(instance_id, id, response.status, &response.body);
    }
    Ok(response)
}

/// Serves one filesystem operation, or reports the backend that cannot hold a machine for it.
///
/// A backend answering `Unsupported` here is not failing: it has no machine a later call could
/// address, so there is nothing for the operation to reach. That is exactly what the missing
/// capability names, so it is reported as the capability rather than as a backend fault.
fn filesystem<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance_id: soma::InstanceId,
    file_operation: FilesystemOperation,
    body: &[u8],
) -> Result<Response, ApiError> {
    let request = parse::<FilesystemBody>(body)?.into_facade(instance_id, file_operation)?;
    let outcome = facade.file(request).map_err(|failure| {
        if matches!(
            failure,
            soma::ManagedFailure::Backend(soma::BackendFailureKind::Unsupported)
        ) {
            MissingCapability::GuestFilesystemTransfer.error()
        } else {
            managed_error(&failure)
        }
    })?;
    let report = FilesystemReport::new(outcome.instance_id, outcome.operation, &outcome.answer);
    Ok(success(200, "sandbox.filesystem", &report, None))
}

/// Serves one terminal operation, or reports the backend that cannot hold a session for it.
///
/// A backend answering `Unsupported` here is not failing: it has no machine a second request
/// could address, so a session opened by the first one would not exist by the time the second
/// arrived. That is exactly what the missing capability names.
fn terminal<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance_id: soma::InstanceId,
    terminal_operation: TerminalOperation,
    body: &[u8],
) -> Result<Response, ApiError> {
    let request =
        optional_body::<TerminalBody>(body)?.into_facade(instance_id, terminal_operation)?;
    let outcome = facade.terminal(request).map_err(|failure| {
        if matches!(
            failure,
            soma::ManagedFailure::Backend(soma::BackendFailureKind::Unsupported)
        ) {
            MissingCapability::GuestTerminalSession.error()
        } else {
            managed_error(&failure)
        }
    })?;
    let report = TerminalReport::new(outcome.instance_id, outcome.operation, &outcome.answer);
    Ok(success(200, "sandbox.terminal", &report, None))
}
