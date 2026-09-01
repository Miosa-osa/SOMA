//! Whether a Machine one backend launches outlives the process that launched it.
//!
//! This is a property of the backend rather than of any one Launch, and it is the difference
//! between an instance identity a later command can use and one that names a Machine which died
//! with the process that reported it. It lives beside the backends rather than inside any of them
//! because every public surface has to ask the same question before it hands an identity back.

use soma::BackendKind;

/// Whether a Machine one backend launches is still addressable once the launching process is
/// gone.
///
/// This is the difference between an instance identity a later command can use and one that
/// names a Machine which died with the process that reported it. A surface that hands an
/// identity back has to know which it is holding, because reporting a launch as ready without
/// it is reporting a success no second process can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineHosting {
    /// The Machine and its guest session are resident in the launching process.
    LaunchingProcess,
    /// The Machine is hosted outside the launching process and outlives it.
    OutlivesProcess,
}

/// How long a Machine launched on `backend` remains reachable.
#[must_use]
pub const fn machine_hosting(backend: BackendKind) -> MachineHosting {
    match backend {
        // macOS has no machine host yet: the machine and its guest session are held by the
        // process that launched them, so nothing survives it for a second command to reach.
        // This is the only backend left that hands back an identity no later command can use.
        BackendKind::MacosVirtualization => MachineHosting::LaunchingProcess,
        // The rest keep the machine somewhere the launching process is not, by two different
        // routes: Docker registers the container with the host daemon under a name derived from
        // the Instance, and a KVM managed Launch starts a host process answering on a socket
        // named by the Instance. Either way a later command reaches the machine by identity.
        BackendKind::DockerContainer | BackendKind::Remote | BackendKind::LinuxKvm => {
            MachineHosting::OutlivesProcess
        }
    }
}
