mod prepared;
mod prewarm;
mod support;
mod volume;
mod volumes;
use support::{control_body, optional_body, parse, refuse, success};
use volume::binding as volume_binding;

use soma::ExecutionReceipt;

use crate::{
    capability::MissingCapability,
    envelope::{ApiError, Envelope, failure_body_with_result, render},
    facade::SandboxFacade,
    failure::managed_error,
    filesystem::{FilesystemBody, FilesystemReport},
    http::{request::Request, response::Response},
    report::{
        CommandReport, InspectionReport, OutputBytes, SandboxListReport, SandboxReport,
        VolumeReady, command_status,
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
    if let Err(error) = identify(request.header(TENANT_HEADER)) {
        return refuse("request", error);
    }
    match resolve(request) {
        Ok(route) => {
            let operation = route.operation();
            dispatch(facade, route, &request.body).unwrap_or_else(|error| refuse(operation, error))
        }
        Err(error) => refuse("request", error),
    }
}

fn dispatch<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    route: Route,
    body: &[u8],
) -> Result<Response, ApiError> {
    let operation = route.operation();
    match route {
        Route::CreateSandbox => create(facade, body),
        Route::ListSandboxes => {
            let listing = facade.list().map_err(|f| managed_error(&f))?;
            Ok(success(
                200,
                operation,
                &SandboxListReport::from_listing(&listing),
                None,
            ))
        }
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
                },
                Some(&snapshot.receipt),
            ))
        }
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
        // Served by the connection before a facade is opened, through `handle_prewarm`. This
        // function serves sandboxes, so only that one path can change what is prepared.
        Route::Prewarm(_) => Err(ApiError::not_found()),
        Route::PrunePrepared => prepared::prune(facade, operation, body),
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
            volumes::serve(facade, volume_operation, path_volume, body)
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

fn create<F: SandboxFacade + ?Sized>(facade: &mut F, body: &[u8]) -> Result<Response, ApiError> {
    let body = parse::<CreateSandboxBody>(body)?;
    let named_volume = body.volume_id.clone();
    let (instance_id, request) = body.into_facade()?;
    // A start that names its volume binds that file and no other: the host resolves it from the
    // volume this Instance owns, and a disagreement between the two is refused rather than
    // resolved by guessing which is right.
    let (request, volume, fsck) = match named_volume.as_deref() {
        Some(named) => {
            let (attachment, fsck) = volume_binding(facade, &instance_id, named)?;
            (
                request.with_volume(attachment),
                Some(VolumeReady {
                    volume_id: named.to_ascii_lowercase(),
                    state: "attached",
                    device: VOLUME_DEVICE,
                }),
                Some(fsck),
            )
        }
        None => (request, None, None),
    };
    // Refused before a machine is built rather than answered 201 with an identity that dies
    // with this connection. A caller that keeps the identity and returns is the whole point of
    // the create route, and there would be nothing here for it to return to.
    if !facade.hosts_addressable_sandboxes() {
        return Err(MissingCapability::DurableMachineHosting.error());
    }
    let outcome = facade.launch(request).map_err(|f| managed_error(&f))?;
    // A created sandbox answers 201, matching what a provider contract expects of a create call
    // that produced a new addressable resource.
    Ok(Response::new(
        201,
        render(&Envelope::success(
            "sandbox.create",
            &SandboxReport {
                instance_id: outcome.instance_id,
                state: "ready",
                volume,
                fsck,
            },
            Some(&outcome.receipt),
        )),
    ))
}

fn lifecycle(
    operation: &'static str,
    state: &'static str,
    instance_id: &soma::InstanceId,
    receipt: &ExecutionReceipt,
) -> Response {
    success(
        200,
        operation,
        &SandboxReport::new(instance_id.clone(), state),
        Some(receipt),
    )
}
