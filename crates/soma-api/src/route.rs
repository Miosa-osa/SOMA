use soma::InstanceId;

use crate::{
    envelope::ApiError,
    http::request::{Method, Request},
    wire::path_instance_id,
};

/// The filesystem operations the provider contract expects a sandbox service to expose.
///
/// Each is one call reaching one guest operation. They are named on the path rather than in the
/// body because a caller that asked for a write and a caller that asked for a removal are doing
/// different things to the same resource, and a proxy or a log should be able to tell them apart
/// without reading the body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesystemOperation {
    Read,
    Write,
    List,
    Exists,
    Remove,
    MakeDirectory,
}

impl FilesystemOperation {
    /// The operation's own name, as the response envelope reports it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::List => "list",
            Self::Exists => "exists",
            Self::Remove => "remove",
            Self::MakeDirectory => "mkdir",
        }
    }
}

/// The terminal operations the provider contract expects a sandbox service to expose.
///
/// A terminal is a stream and these are calls, deliberately. The session lives in the sandbox for
/// as long as the sandbox does, and a caller drives it with bounded requests: nothing here holds a
/// connection open, so it works over the same HTTP the rest of this service is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalOperation {
    Open,
    Write,
    Read,
    Resize,
    Close,
}

impl TerminalOperation {
    /// The operation's own name, as the response envelope reports it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Write => "write",
            Self::Read => "read",
            Self::Resize => "resize",
            Self::Close => "close",
        }
    }
}

/// One resolved route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Route {
    CreateSandbox,
    ListSandboxes,
    GetSandbox(InstanceId),
    StopSandbox(InstanceId),
    DestroySandbox(InstanceId),
    RunCommand(InstanceId),
    Filesystem(InstanceId, FilesystemOperation),
    Terminal(InstanceId, TerminalOperation),
}

impl Route {
    /// Names the operation for the response envelope, mirroring the CLI's command names.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::CreateSandbox => "sandbox.create",
            Self::ListSandboxes => "sandbox.list",
            Self::GetSandbox(_) => "sandbox.get",
            Self::StopSandbox(_) => "sandbox.stop",
            Self::DestroySandbox(_) => "sandbox.destroy",
            Self::RunCommand(_) => "sandbox.command",
            Self::Filesystem(_, _) => "sandbox.filesystem",
            Self::Terminal(_, _) => "sandbox.terminal",
        }
    }
}

/// Resolves a request to a route.
///
/// A path that exists under a different method returns 405 rather than 404, so a client with a
/// wrong verb is told its verb is wrong instead of hunting for a path that is in fact present.
///
/// # Errors
///
/// Returns a 404 refusal for an unknown path and a 405 refusal for a known path under an
/// unaccepted method.
pub fn resolve(request: &Request) -> Result<Route, ApiError> {
    let segments = request.segments();
    match segments.as_slice() {
        ["v1", "sandboxes"] => match request.method {
            Method::Post => Ok(Route::CreateSandbox),
            Method::Get => Ok(Route::ListSandboxes),
            _ => Err(ApiError::method_not_allowed()),
        },
        ["v1", "sandboxes", instance] => {
            let instance_id = path_instance_id(instance)?;
            match request.method {
                Method::Get => Ok(Route::GetSandbox(instance_id)),
                Method::Delete => Ok(Route::DestroySandbox(instance_id)),
                _ => Err(ApiError::method_not_allowed()),
            }
        }
        ["v1", "sandboxes", instance, "stop"] => {
            post_only(request, Route::StopSandbox(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "commands"] => {
            post_only(request, Route::RunCommand(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "filesystem", operation] => {
            let route = Route::Filesystem(
                path_instance_id(instance)?,
                filesystem_operation(operation)?,
            );
            post_only(request, route)
        }
        ["v1", "sandboxes", instance, "terminal", operation] => {
            let route =
                Route::Terminal(path_instance_id(instance)?, terminal_operation(operation)?);
            post_only(request, route)
        }
        _ => Err(ApiError::not_found()),
    }
}

fn post_only(request: &Request, route: Route) -> Result<Route, ApiError> {
    if request.method == Method::Post {
        Ok(route)
    } else {
        Err(ApiError::method_not_allowed())
    }
}

fn filesystem_operation(operation: &str) -> Result<FilesystemOperation, ApiError> {
    match operation {
        "read" => Ok(FilesystemOperation::Read),
        "write" => Ok(FilesystemOperation::Write),
        "list" => Ok(FilesystemOperation::List),
        "exists" => Ok(FilesystemOperation::Exists),
        "remove" => Ok(FilesystemOperation::Remove),
        "mkdir" => Ok(FilesystemOperation::MakeDirectory),
        _ => Err(ApiError::not_found()),
    }
}

fn terminal_operation(operation: &str) -> Result<TerminalOperation, ApiError> {
    match operation {
        "open" => Ok(TerminalOperation::Open),
        "write" => Ok(TerminalOperation::Write),
        "read" => Ok(TerminalOperation::Read),
        "resize" => Ok(TerminalOperation::Resize),
        "close" => Ok(TerminalOperation::Close),
        _ => Err(ApiError::not_found()),
    }
}
