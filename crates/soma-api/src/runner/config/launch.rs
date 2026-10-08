//! What a create launches: the image, the shapes `size` may select, and the idle timeout.
//!
//! A create names a size, not a machine. An absent `size`, or `xs`, selects [`LaunchConfig::shape`],
//! which is the shape the host was sized and prewarmed for. `large` selects [`LaunchConfig::large`],
//! which a runner only has when an operator configured one. Keeping both shapes here rather than on
//! the create path is what makes "this host serves large machines" a property of the host rather
//! than a request a caller can invent.

use serde::Deserialize;
use soma::{Capabilities, MachineShape};

/// The idle timeout a create gets when neither the request nor the tenant names one.
const DEFAULT_TIMEOUT_SECONDS: u64 = crate::runner::idle::DEFAULT_IDLE_TIMEOUT_SECONDS;

/// The vCPUs a large machine gets when an operator names none: the machine contract v2 ceiling.
const DEFAULT_LARGE_VCPU_COUNT: u16 = 8;
/// The guest RAM a large machine gets when an operator names none: the contract v2 ceiling.
const DEFAULT_LARGE_MEMORY_MIB: u64 = 16 * 1024;
/// The writable storage a large machine gets when an operator names none.
///
/// Twenty gigabytes is the shape the large workload was measured at: a checkout, a package
/// install of a few thousand modules, and a type check fit with room for their caches.
const DEFAULT_LARGE_STORAGE_MIB: u64 = 20 * 1024;

/// What a create launches, and what the compact create answer reports about it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchConfig {
    pub image: String,
    /// The shape a create gets when it names no size, or names `xs`.
    ///
    /// This is the machine the host is sized and prewarmed for, so it is the one shape a create
    /// may always have. A request for another size is refused unless [`Self::large`] names one.
    pub shape: MachineShape,
    /// The shape a create gets when it names `size: "large"`, absent on a runner that serves
    /// exactly one shape.
    ///
    /// A runner that serves large machines says so here, and a create that asks for one is then
    /// served it: the size resolves to this shape, and every later check -- `cpu_count`,
    /// `memory_mb`, the compact answer, the launch itself -- is made against it rather than
    /// against [`Self::shape`]. On a runner without this block a `size: "large"` create is
    /// answered with the same 400 an unknown size gets, so two hosts can never disagree about
    /// what `large` means.
    #[serde(default)]
    pub large: Option<LargeShape>,
    /// The template id the compact answer reports, as the standard path's `template_id`.
    pub template_id: String,
    #[serde(default = "default_timeout_seconds")]
    pub default_timeout_seconds: u64,
}

/// The large machine a `size: "large"` create selects.
///
/// Every field defaults, so an operator names only what they change away from the shape the large
/// workload was measured at. The defaults are that shape: eight vCPUs, sixteen gigabytes of guest
/// RAM, and twenty gigabytes of writable storage. They are also the machine contract v2 ceiling,
/// so a Generation built for this shape is the one that boots it, and a Generation built for any
/// other is refused rather than served as a smaller machine.
///
/// [`Self::public_egress`] is the network knob. On, the machine is built with the platform's
/// public-internet policy: the backend leases an egress bundle from this host's broker on the
/// launch path, and the guest resolves and fetches through it. Off, the same machine is built
/// with the isolated policy and reaches nothing. There is no third value, and a machine is never
/// reported as having a network it was not given.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LargeShape {
    #[serde(default = "default_large_vcpu_count")]
    pub vcpu_count: u16,
    #[serde(default = "default_large_memory_mib")]
    pub memory_mib: u64,
    #[serde(default = "default_large_storage_mib")]
    pub storage_mib: u64,
    /// Whether a large machine reaches the public internet. On unless an operator says off.
    #[serde(default = "default_public_egress")]
    pub public_egress: bool,
}

impl LargeShape {
    /// The machine shape this block describes.
    ///
    /// # Panics
    ///
    /// Panics only if a configured shape is invalid, which the configuration loader refuses long
    /// before a create can reach here.
    #[must_use]
    pub fn machine_shape(&self) -> MachineShape {
        let capabilities = if self.public_egress {
            Capabilities::isolated().with_network_access()
        } else {
            Capabilities::isolated()
        };
        MachineShape::new(self.vcpu_count, self.memory_mib, self.storage_mib)
            .expect("a validated large shape is a valid machine shape")
            .with_capabilities(capabilities)
    }
}

const fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

const fn default_large_vcpu_count() -> u16 {
    DEFAULT_LARGE_VCPU_COUNT
}

const fn default_large_memory_mib() -> u64 {
    DEFAULT_LARGE_MEMORY_MIB
}

const fn default_large_storage_mib() -> u64 {
    DEFAULT_LARGE_STORAGE_MIB
}

const fn default_public_egress() -> bool {
    true
}
