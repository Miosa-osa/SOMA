//! Opening one Instance's volume for the start that is binding a machine to it.
//!
//! It lives apart from the request-shaped surface because it is not a route: the control plane
//! never opens a file, it starts an Instance and the host decides which file that Instance owns.

use std::path::PathBuf;

use soma_hostd::{InstanceId as HostInstance, Reply, Request};

use super::{FsckReport, VolumeClient, VolumeFailure, exchange};

/// One Instance's volume, opened for the start that is binding a machine to it.
///
/// The path is the host's own, derived by the store from the volume this Instance owns, so a
/// caller never names a file. The repair is this start's receipt: the volume is checked on the
/// host, offline, before any machine holds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeBinding {
    /// The volume bound to the Instance.
    pub volume: String,
    /// The file the machine binds over its placeholder.
    pub path: PathBuf,
    /// The disk class in GiB.
    pub size_class_gib: u32,
    /// The repair this start ran, in the shape a start reports.
    pub fsck: FsckReport,
}

impl FsckReport {
    /// The receipt one repair outcome code and exit code describe.
    ///
    /// The outcome vocabulary is the host's: 0 is a repair that did not run, 1 a filesystem that
    /// was already consistent, 2 one this run repaired, and anything else a repair that could
    /// not make the volume usable and was therefore refused before any machine held it.
    #[must_use]
    pub fn from_codes(outcome: u8, exit_code: i8) -> Self {
        Self {
            ran: outcome != 0,
            repaired: outcome == 2,
            exit_code: i32::from(exit_code),
            tool: "e2fsck",
            receipt_id: None,
        }
    }
}

impl VolumeClient {
    /// Opens the volume attached to one Instance, repairing it for the start that asked.
    ///
    /// It is the start's half of the volume lifecycle rather than a route: a caller starts an
    /// Instance and the host decides which file that Instance owns.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeFailure::Unconfigured`] when no socket is set, the daemon's own refusal
    /// when the Instance has no attached volume or the volume cannot be made usable, and a
    /// transport failure when the frames cannot be exchanged.
    pub fn binding(&self, instance: &str) -> Result<VolumeBinding, VolumeFailure> {
        let Some(socket) = self.socket.as_deref() else {
            return Err(VolumeFailure::Unconfigured);
        };
        let instance = HostInstance::from_hex(instance).map_err(|_| VolumeFailure::Malformed)?;
        match exchange(socket, &Request::VolumeOpen { instance })? {
            Reply::VolumeOpened {
                volume,
                size_class_gib,
                repair,
                exit_code,
                path,
            } => Ok(VolumeBinding {
                volume: volume.hex(),
                path: PathBuf::from(path),
                size_class_gib,
                fsck: FsckReport::from_codes(repair, exit_code),
            }),
            Reply::Failed(code) => Err(VolumeFailure::Refused(code)),
            _ => Err(VolumeFailure::Malformed),
        }
    }
}
