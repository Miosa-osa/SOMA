use serde::{Serialize, de::DeserializeOwned};
use soma::ExecutionReceipt;

use crate::{
    capability::MissingCapability,
    envelope::{ApiError, Envelope, failure_body, render},
    facade::SandboxFacade,
    failure::managed_error,
    http::{request::Request, response::Response},
    report::{CommandReport, InspectionReport, OutputBytes, SandboxReport, command_status},
    route::{Route, resolve},
    tenant::{TENANT_HEADER, identify},
    wire::{ControlBody, CreateSandboxBody, RunCommandBody},
};

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
        // Enumeration and filesystem transfer are refused with the capability the engine lacks.
        // Returning an empty list here would be indistinguishable from a tenant owning nothing,
        // which is the one answer a caller must never be given by mistake.
        Route::ListSandboxes => Err(MissingCapability::SandboxEnumeration.error()),
        Route::Filesystem(_, _) => Err(MissingCapability::GuestFilesystemTransfer.error()),
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
            let request = control_body(body)?.into_stop(instance_id)?;
            let outcome = facade.stop(request).map_err(|f| managed_error(&f))?;
            Ok(lifecycle(
                operation,
                "stopped",
                &outcome.instance_id,
                &outcome.receipt,
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
    }
}

fn create<F: SandboxFacade + ?Sized>(facade: &mut F, body: &[u8]) -> Result<Response, ApiError> {
    let (_, request) = parse::<CreateSandboxBody>(body)?.into_facade()?;
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
        &SandboxReport {
            instance_id: instance_id.clone(),
            state,
        },
        Some(receipt),
    )
}

fn success<T: Serialize>(
    status: u16,
    operation: &'static str,
    result: &T,
    receipt: Option<&ExecutionReceipt>,
) -> Response {
    Response::new(
        status,
        render(&Envelope::success(operation, result, receipt)),
    )
}

fn refuse(operation: &'static str, error: ApiError) -> Response {
    Response::new(error.status(), failure_body(operation, error))
}

/// Parses a required JSON body.
///
/// The facade's wire types reject unknown fields, so a caller that misspells a field is told so
/// rather than having its intent silently dropped.
fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::invalid("the request body is not a valid document for this route"))
}

/// Parses an optional control body, treating an absent body as an empty one.
///
/// Control routes need nothing from the caller beyond the path, so requiring `{}` would be
/// ceremony; supplying an operation id stays available for a caller that wants to choose it.
fn control_body(body: &[u8]) -> Result<ControlBody, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(ControlBody::default());
    }
    parse(body)
}
