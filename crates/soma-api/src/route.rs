use soma::InstanceId;

use crate::{
    envelope::ApiError,
    http::request::{Method, Request},
    volumes::VolumeOperation,
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
    /// Guest API level 2: kind, size, mode, and modification time.
    Stat,
    /// Guest API level 2: atomic move.
    Rename,
    /// Guest API level 2: a byte range of a file.
    ReadRange,
    /// Guest API level 2: a byte range written into a file.
    WriteRange,
    /// Guest API level 2: permission bits.
    Chmod,
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
            Self::Stat => "stat",
            Self::Rename => "rename",
            Self::ReadRange => "read_range",
            Self::WriteRange => "write_range",
            Self::Chmod => "chmod",
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
    PauseSandbox(InstanceId),
    ResumeSandbox(InstanceId),
    CheckpointSandbox(InstanceId),
    DiscardCheckpoint(InstanceId),
    RestoreSandbox(InstanceId),
    ForkSandbox(InstanceId),
    /// Suspend, resume and discard a Suspension (ADR 0034).
    SuspendSandbox(InstanceId),
    ResumeSuspension(String),
    DestroySuspension(String),
    Capacity,
    /// `GET` reads, `PUT` sets, a sandbox's automatic sizing.
    Autosize(InstanceId, bool),
    DestroySandbox(InstanceId),
    RunCommand(InstanceId),
    Filesystem(InstanceId, FilesystemOperation),
    Terminal(InstanceId, TerminalOperation),
    /// A raw byte stream to a guest port; served by the connection, not by a handler.
    Tunnel(InstanceId, u16),
    /// `GET` reads, `PATCH` changes, what a running sandbox has.
    Resources(InstanceId, bool),
    /// `GET` reads the desired prewarm set and what exists, `PUT` replaces the set (ADR 0052).
    ///
    /// Served by the connection, not by a handler: it needs no sandbox facade, so a burst that
    /// holds every facade cannot also block a caller trying to change what is prepared.
    Prewarm(bool),
    /// `POST` plans, and with `apply` performs, the release of prepared Generations (ADR 0054).
    ///
    /// Served with a facade, because whether a Generation is still in use is what the durable
    /// store's records say.
    PrunePrepared,
    /// One durable data volume operation (ADR 0053 Step A).
    ///
    /// The read carries its volume id on the path and no body; the others carry a body and no id.
    Volume(VolumeOperation, Option<String>),
    /// `GET` reads the API version, the build identity, and the protocols this service speaks.
    ///
    /// Served by the connection, not by a handler: it needs no sandbox facade.
    Version,
    /// Moving a capture to and from off-host storage; the platform tenant only (ADR 0070).
    CheckpointTransfer(TransferRoute),
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
            Self::PauseSandbox(_) => "sandbox.pause",
            Self::ResumeSandbox(_) => "sandbox.resume",
            Self::CheckpointSandbox(_) => "sandbox.checkpoint",
            Self::DiscardCheckpoint(_) => "sandbox.checkpoint.discard",
            Self::RestoreSandbox(_) => "sandbox.restore",
            Self::ForkSandbox(_) => "sandbox.fork",
            Self::SuspendSandbox(_) => "sandbox.suspend",
            Self::ResumeSuspension(_) => "suspension.resume",
            Self::DestroySuspension(_) => "suspension.destroy",
            Self::Capacity => "host.capacity",
            Self::Autosize(..) => "sandbox.autosize",
            Self::DestroySandbox(_) => "sandbox.destroy",
            Self::RunCommand(_) => "sandbox.command",
            Self::Filesystem(_, _) => "sandbox.filesystem",
            Self::Terminal(_, _) => "sandbox.terminal",
            Self::Tunnel(_, _) => "sandbox.tunnel",
            Self::Resources(_, false) => "sandbox.resources",
            Self::Resources(_, true) => "sandbox.resize",
            Self::Prewarm(false) => "prewarm.status",
            Self::Prewarm(true) => "prewarm.set",
            Self::PrunePrepared => "prepared.prune",
            Self::Volume(operation, _) => operation.name(),
            Self::Version => crate::version::VERSION_OPERATION,
            Self::CheckpointTransfer(transfer) => transfer.operation(),
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
        ["v1", "version"] => get_only(request, Route::Version),
        ["v1", "checkpoints", rest @ ..] => resolve_transfer(request.method, rest),
        ["v1", "prewarm"] => match request.method {
            Method::Get => Ok(Route::Prewarm(false)),
            Method::Put => Ok(Route::Prewarm(true)),
            _ => Err(ApiError::method_not_allowed()),
        },
        ["v1", "prepared", "prune"] => post_only(request, Route::PrunePrepared),
        ["v1", "capacity"] => get_only(request, Route::Capacity),
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
        ["v1", "sandboxes", instance, "pause"] => {
            post_only(request, Route::PauseSandbox(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "resume"] => {
            post_only(request, Route::ResumeSandbox(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "checkpoints"] => post_only(
            request,
            Route::CheckpointSandbox(path_instance_id(instance)?),
        ),
        ["v1", "sandboxes", instance, "checkpoints", "discard"] => post_only(
            request,
            Route::DiscardCheckpoint(path_instance_id(instance)?),
        ),
        ["v1", "sandboxes", instance, "restore"] => {
            post_only(request, Route::RestoreSandbox(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "fork"] => {
            post_only(request, Route::ForkSandbox(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "stop"] => {
            post_only(request, Route::StopSandbox(path_instance_id(instance)?))
        }
        ["v1", "sandboxes", instance, "suspend"] => {
            post_only(request, Route::SuspendSandbox(path_instance_id(instance)?))
        }
        ["v1", "suspensions", id, "resume"] => {
            post_only(request, Route::ResumeSuspension((*id).to_ascii_lowercase()))
        }
        ["v1", "suspensions", id] => match request.method {
            Method::Delete => Ok(Route::DestroySuspension((*id).to_ascii_lowercase())),
            _ => Err(ApiError::method_not_allowed()),
        },
        ["v1", "sandboxes", instance, "autosize"] => autosize(request, instance),
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
        ["v1", "sandboxes", instance, "resources"] => {
            let instance_id = path_instance_id(instance)?;
            match request.method {
                Method::Get => Ok(Route::Resources(instance_id, false)),
                Method::Patch => Ok(Route::Resources(instance_id, true)),
                _ => Err(ApiError::method_not_allowed()),
            }
        }
        ["v1", "sandboxes", instance, "tunnel", port] => {
            tunnel(request.method, path_instance_id(instance)?, port)
        }
        ["v1", "sandboxes", instance, "terminal", operation] => {
            let route =
                Route::Terminal(path_instance_id(instance)?, terminal_operation(operation)?);
            post_only(request, route)
        }
        // The named volume operations come before the bare id, so `attach` is an operation and
        // never read as a volume identity.
        ["v1", "volumes", "attach"] => {
            post_only(request, Route::Volume(VolumeOperation::Attach, None))
        }
        ["v1", "volumes", "detach"] => {
            post_only(request, Route::Volume(VolumeOperation::Detach, None))
        }
        ["v1", "volumes", "destroy"] => {
            post_only(request, Route::Volume(VolumeOperation::Destroy, None))
        }
        ["v1", "volumes", id] => match request.method {
            Method::Get => Ok(Route::Volume(VolumeOperation::Read, Some((*id).to_owned()))),
            _ => Err(ApiError::method_not_allowed()),
        },
        _ => Err(ApiError::not_found()),
    }
}

fn resolve_transfer(method: Method, rest: &[&str]) -> Result<Route, ApiError> {
    transfer::resolve(method, rest).map(Route::CheckpointTransfer)
}

mod methods;
mod operations;
pub mod transfer;
use methods::{autosize, get_only, post_only, tunnel};
use operations::{filesystem_operation, terminal_operation};
pub use transfer::{TransferArtifact, TransferRoute};
