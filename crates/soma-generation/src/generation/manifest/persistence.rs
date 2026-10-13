//! The durable data volume group of ADR 0053 Step A, which only compiler policy 4 writes.
//!
//! Keeping it in its own module is what holds the manifest encoder inside the repository's
//! source-line budget; the group is otherwise an ordinary part of the manifest.

/// The largest number of persistent paths one manifest may declare.
pub(crate) const MAX_PERSISTENCE_PATHS: usize = 4;

/// The disk classes a Step A durable volume may be created at, in GiB.
///
/// The class is the volume device's capacity, so it is fixed by the Generation that declares the
/// device rather than chosen by a launch: a start that asked for another class would be served by
/// a machine whose captured device state disagrees with the file behind it.
pub const VOLUME_CLASSES_GIB: [u32; 3] = [10, 20, 40];

/// The class a Step A Generation declares when its profile names none.
pub const DEFAULT_VOLUME_CLASS_GIB: u32 = 10;

/// Whether one value is a class a Step A volume may be created at.
#[must_use]
pub const fn is_volume_class(gib: u32) -> bool {
    let mut index = 0;
    while index < VOLUME_CLASSES_GIB.len() {
        if VOLUME_CLASSES_GIB[index] == gib {
            return true;
        }
        index += 1;
    }
    false
}

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
    /// The disk class the volume is created at, in GiB.
    ///
    /// It is the device's capacity, so a capture, a restore, and every launch of this Generation
    /// agree about the file without any of them being told again. Zero on a binding that declares
    /// no volume, which is refused rather than reinterpreted if it carries a class.
    pub size_class_gib: u32,
}
