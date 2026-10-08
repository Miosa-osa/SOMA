//! The recipe for one captured machine, and the fixed parameters its capture point is described
//! by: the image to build from, the shape and contract to build at, the console line the guest
//! agent parks on, and how long each wait may take.
//!
//! Its own file because the fixture that builds from a recipe is close to the repository's source
//! ceiling, and because a recipe is data: two of them exist, one per contract in use.

use std::time::Duration;

use soma_kvm::MachineContract;

/// The image the captured Generation is built from.
pub const IMAGE: &str = "node:22";
/// Environment variable naming a pre-exported OCI layout for it.
pub const LAYOUT_VAR: &str = "SOMA_OCI_NODE_LAYOUT";
/// Guest RAM of the captured machine, matching the cold-boot evidence for `node:22`.
pub const MEMORY_MIB: u64 = 1024;
/// Writable class of the captured machine; every restore clones a head of this size.
pub const STORAGE_MIB: u64 = 256;

/// The image, shape, and contract one captured machine is built from.
pub struct Recipe {
    /// The image the Generation is built from.
    pub image: &'static str,
    /// Environment variable naming a pre-exported OCI layout for it.
    pub layout_var: &'static str,
    /// Name of this recipe's scratch tree under the run's scratch root.
    pub scratch: &'static str,
    /// vCPUs the captured machine runs.
    pub vcpus: u16,
    /// Guest RAM in MiB.
    pub memory_mib: u64,
    /// Writable class in MiB.
    pub storage_mib: u64,
    /// The contract the Generation is compiled, captured, and restored under.
    pub contract: MachineContract,
}

/// The single-vCPU `node:22` machine every test written before contract v2 shares.
pub const NODE22: Recipe = Recipe {
    image: IMAGE,
    layout_var: LAYOUT_VAR,
    scratch: "node22",
    vcpus: 1,
    memory_mib: MEMORY_MIB,
    storage_mib: STORAGE_MIB,
    contract: MachineContract::V1,
};

/// The eight-vCPU, sixteen-gigabyte machine contract v2 exists for.
///
/// `BusyBox` rather than `node:22`: the claim under test is the shape, and the smaller image keeps
/// the capture and every restore cheap. `storage_mib` stays at the same writable class, because
/// nothing in the shape claim depends on it and a larger head costs the restore time.
pub const BUSYBOX_V2: Recipe = Recipe {
    image: "busybox:stable-musl",
    layout_var: "SOMA_OCI_BUSYBOX_LAYOUT",
    scratch: "busybox-v2",
    vcpus: 8,
    memory_mib: 16 * 1024,
    storage_mib: STORAGE_MIB,
    contract: MachineContract::V2,
};

/// Free space a contract v2 capture needs before it starts.
///
/// The capture walk writes the whole memory object, so a sixteen-gigabyte machine stages sixteen
/// gigabytes of image alongside its root, its overlay, and its state, and a run that starts with
/// less fails deep inside the walk instead of at the door.
pub const V2_REQUIRED_FREE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// The context identifier the captured machine holds; every restore is assigned another.
pub const CAPTURE_CID: u32 = 3;
/// The exact console line the pinned guest agent prints at the disconnected repair point.
pub const REPAIR_POINT_LINE: &[u8] = b"soma-guest-agent: awaiting launch material";
/// How long the guest may take to reach the repair point.
pub const REPAIR_POINT_DEADLINE: Duration = Duration::from_secs(120);
/// How long vCPU 0 may take to leave `KVM_RUN` after the capture kicks it.
pub const PAUSE_GRACE: Duration = Duration::from_secs(10);
