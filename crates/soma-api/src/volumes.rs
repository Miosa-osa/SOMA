//! The durable data volume surface of ADR 0053 Step A, as a client of the host daemon.
//!
//! This module owns no volume state. The store lives inside `soma-hostd`, and this is the client
//! that encodes the daemon's own frames, exchanges them over the host control socket, and answers
//! with the outcome the daemon sent or its stable refusal.
//! When no socket is configured every operation reports the missing capability rather than an
//! invented answer, which is the rule the rest of this service already follows.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use soma_hostd::client::HostClient;
use soma_hostd::{
    AttachFrame, DetachFrame, GenerationId, InstanceId as HostInstance,
    OperationId as HostOperation, Reply, Request, RequestFingerprint, VolumeId as HostVolume,
};

pub use binding::VolumeBinding;
use refusal::state_name;

mod binding;
mod refusal;
#[cfg(test)]
mod tests;

/// The environment variable naming the host control socket. Unset means no volume is served.
pub const HOSTD_SOCKET_ENV: &str = "SOMA_HOSTD_SOCKET";
/// The device a Step A volume is bound to.
pub const VOLUME_DEVICE: &str = "vdc";

/// One volume operation, named on the path like every other operation in this service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeOperation {
    Attach,
    Detach,
    Destroy,
    Read,
}

impl VolumeOperation {
    /// The operation name the response envelope reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Attach => "volume.attach",
            Self::Detach => "volume.detach",
            Self::Destroy => "volume.destroy",
            Self::Read => "volume.read",
        }
    }

    /// Whether the operation carries a request body.
    #[must_use]
    pub const fn has_body(self) -> bool {
        !matches!(self, Self::Read)
    }
}

/// The request body of one volume operation, as the control plane sends it.
#[derive(Clone, Debug, Deserialize)]
pub struct VolumeBody {
    pub operation_id: String,
    pub instance_id: String,
    pub volume_id: String,
    pub request_fingerprint: String,
    #[serde(default)]
    pub size_class_gib: u32,
    #[serde(default)]
    pub generation_id: Option<String>,
    #[serde(default)]
    pub tenant: Option<String>,
}

/// The repair receipt one attach or one start carries.
///
/// The durable receipt's own identifier is host-side and is not on the wire yet, so it is null
/// rather than invented.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FsckReport {
    pub ran: bool,
    pub repaired: bool,
    pub exit_code: i32,
    pub tool: &'static str,
    pub receipt_id: Option<String>,
}

/// What one volume operation answered.
#[derive(Clone, Debug, Serialize)]
pub struct VolumeReport {
    pub volume_id: String,
    pub operation: &'static str,
    pub device: &'static str,
    pub state: &'static str,
    pub ready: bool,
    pub fsck: Option<FsckReport>,
}

/// Why a volume operation could not be answered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeFailure {
    /// No host control socket is configured, so no volume can be served.
    Unconfigured,
    /// The daemon refused with a stable code.
    Refused(u16),
    /// The request named something the wire cannot carry.
    Malformed,
    /// The frames could not be exchanged.
    Transport,
}

/// The socket path one environment value selects; absent or blank means unconfigured.
#[must_use]
pub fn socket_from(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// A client of the host daemon's volume operations.
pub struct VolumeClient {
    socket: Option<PathBuf>,
}

impl VolumeClient {
    /// Reads the control socket from the environment; unset or blank means unconfigured.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            socket: socket_from(std::env::var_os(HOSTD_SOCKET_ENV)),
        }
    }

    /// A client for one exact socket path, or one that serves nothing when it is absent.
    #[must_use]
    pub fn at(socket: Option<PathBuf>) -> Self {
        Self { socket }
    }

    /// Whether a host control socket is configured.
    #[must_use]
    pub const fn configured(&self) -> bool {
        self.socket.is_some()
    }

    /// Performs one volume operation against the host daemon.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeFailure::Unconfigured`] when no socket is set, [`VolumeFailure::Malformed`]
    /// when the request cannot be carried, the daemon's own refusal, or a transport failure.
    pub fn call(
        &self,
        operation: VolumeOperation,
        body: &VolumeBody,
    ) -> Result<VolumeReport, VolumeFailure> {
        let Some(socket) = self.socket.as_deref() else {
            return Err(VolumeFailure::Unconfigured);
        };
        let request = request_for(operation, body)?;
        let reply = exchange(socket, &request)?;
        report_for(operation, body, &reply)
    }
}

/// Builds the daemon frame one request carries.
fn request_for(operation: VolumeOperation, body: &VolumeBody) -> Result<Request, VolumeFailure> {
    let operation_id =
        HostOperation::from_hex(&body.operation_id).map_err(|_| VolumeFailure::Malformed)?;
    let fingerprint = RequestFingerprint::from_hex(&body.request_fingerprint)
        .map_err(|_| VolumeFailure::Malformed)?;
    let volume = HostVolume::from_hex(&body.volume_id).map_err(|_| VolumeFailure::Malformed)?;
    if operation == VolumeOperation::Read {
        return Ok(Request::VolumeRead { volume });
    }
    let instance =
        HostInstance::from_hex(&body.instance_id).map_err(|_| VolumeFailure::Malformed)?;
    let frame = DetachFrame {
        operation: operation_id,
        instance,
        volume,
        fingerprint,
    };
    match operation {
        VolumeOperation::Detach => Ok(Request::VolumeDetach(frame)),
        VolumeOperation::Destroy => Ok(Request::VolumeDestroy(frame)),
        _ => {
            let generation = body
                .generation_id
                .as_deref()
                .ok_or(VolumeFailure::Malformed)?;
            let generation =
                GenerationId::from_hex(generation).map_err(|_| VolumeFailure::Malformed)?;
            let tenant = body.tenant.clone().ok_or(VolumeFailure::Malformed)?;
            Ok(Request::VolumeAttach(AttachFrame {
                operation: operation_id,
                instance,
                volume,
                fingerprint,
                size_class_gib: body.size_class_gib,
                generation,
                tenant,
            }))
        }
    }
}

/// Exchanges one frame with the host daemon.
#[cfg(target_os = "linux")]
fn exchange(socket: &std::path::Path, request: &Request) -> Result<Reply, VolumeFailure> {
    let client = HostClient::connect(socket).map_err(|_| VolumeFailure::Transport)?;
    client.call(request).map_err(|_| VolumeFailure::Transport)
}

/// A build with no host daemon socket cannot serve a volume, and says so.
#[cfg(not(target_os = "linux"))]
fn exchange(_socket: &std::path::Path, _request: &Request) -> Result<Reply, VolumeFailure> {
    Err(VolumeFailure::Unconfigured)
}

/// Renders one daemon reply as the report the control plane reads.
fn report_for(
    operation: VolumeOperation,
    body: &VolumeBody,
    reply: &Reply,
) -> Result<VolumeReport, VolumeFailure> {
    match reply {
        Reply::VolumeAttached {
            state,
            repair,
            exit_code,
            ..
        } => Ok(VolumeReport {
            volume_id: body.volume_id.clone(),
            operation: operation.name(),
            device: VOLUME_DEVICE,
            state: state_name(*state),
            ready: *state == 1,
            fsck: Some(FsckReport {
                ran: true,
                repaired: *repair == 2,
                exit_code: i32::from(*exit_code),
                tool: "e2fsck",
                receipt_id: None,
            }),
        }),
        Reply::VolumeDetached { state, .. }
        | Reply::VolumeDestroyed { state, .. }
        | Reply::VolumeInspected { state, .. } => Ok(VolumeReport {
            volume_id: body.volume_id.clone(),
            operation: operation.name(),
            device: VOLUME_DEVICE,
            state: state_name(*state),
            ready: *state == 1,
            fsck: None,
        }),
        Reply::Failed(code) => Err(VolumeFailure::Refused(*code)),
        _ => Err(VolumeFailure::Transport),
    }
}
