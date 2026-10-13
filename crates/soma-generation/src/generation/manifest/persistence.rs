//! The durable data volume group of ADR 0053 Step A, which only compiler policy 4 writes.
//!
//! Keeping it in its own module is what holds the manifest encoder inside the repository's
//! source-line budget; the group is otherwise an ordinary part of the manifest.

/// The largest number of persistent paths one manifest may declare.
pub(crate) const MAX_PERSISTENCE_PATHS: usize = 4;

/// The persistent path set a Step A durable volume carries, in the manifest's order.
///
/// It is the four paths ADR 0053 fixes. Nothing outside this set is persistent in Step A, and
/// `/etc` and package state wait for ADR 0053 Step B.
pub const STEP_A_PERSISTENCE_PATHS: [&str; 4] = [
    "/home/sandbox",
    "/workspace",
    "/var/lib/docker",
    "/opt/user",
];

/// Group 17: the durable data volume of ADR 0053 Step A.
///
/// It is written only from compiler policy 4, so every earlier Generation keeps the exact bytes
/// and the identity it was certified with. A Generation that declares persistence carries the
/// `vdc` volume device and its path list; one that does not carries neither, and a disabled
/// binding that names a path is refused rather than reinterpreted.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersistenceBinding {
    /// Whether this Generation declares a durable data volume.
    pub enabled: bool,
    /// The persistent path set the volume carries, in order.
    pub paths: Vec<String>,
}
