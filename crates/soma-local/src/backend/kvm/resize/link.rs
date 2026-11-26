//! The memory device of whichever machine this process holds: resident, or in a jail.
//!
//! A jailed machine is reached through its worker's control socket, and its cgroup leaf caps the
//! whole jail with a `memory.max`; the resize order in `memory` raises that cap before a plug and
//! lowers it only once the memory is really gone, so the jail is never killed for growing.

use soma::InstanceId;
use soma_jail::CgroupLeaf;
use soma_vmm::sandbox::MemoryLevel;

use super::memory::{LinkError, MemoryLink, ResidentLink};
use crate::backend::kvm::{held::Held, jailed::Jailed};

/// The one memory link a held machine has.
pub(super) enum Link<'a> {
    Resident(ResidentLink<'a>),
    Jailed(JailedLink<'a>),
}

impl<'a> Link<'a> {
    /// The link for `held`, the machine of `instance`.
    pub(super) fn of(held: &'a mut Held, instance: &InstanceId) -> Self {
        match held {
            Held::Resident(session) => Self::Resident(ResidentLink::new(session)),
            Held::Jailed(jailed) => Self::Jailed(JailedLink {
                leaf: jail_leaf(instance),
                jailed,
            }),
        }
    }
}

impl MemoryLink for Link<'_> {
    fn set_requested(&mut self, bytes: u64) -> Result<MemoryLevel, LinkError> {
        match self {
            Self::Resident(link) => link.set_requested(bytes),
            Self::Jailed(link) => link.set_requested(bytes),
        }
    }

    fn level(&mut self) -> Result<MemoryLevel, LinkError> {
        match self {
            Self::Resident(link) => link.level(),
            Self::Jailed(link) => link.level(),
        }
    }

    fn set_memory_max(&mut self, bytes: u64) -> Result<(), ()> {
        match self {
            Self::Resident(link) => link.set_memory_max(bytes),
            Self::Jailed(link) => link.set_memory_max(bytes),
        }
    }
}

/// A machine in a jail: its worker, and the cgroup leaf the jail was built under.
pub(super) struct JailedLink<'a> {
    jailed: &'a mut Jailed,
    leaf: Option<CgroupLeaf>,
}

impl JailedLink<'_> {
    fn exchange(&mut self, requested: Option<u64>) -> Result<MemoryLevel, LinkError> {
        self.jailed
            .memory(requested)
            .map_err(|_| LinkError::Session)?
            .map_err(LinkError::Device)
    }
}

impl MemoryLink for JailedLink<'_> {
    fn set_requested(&mut self, bytes: u64) -> Result<MemoryLevel, LinkError> {
        self.exchange(Some(bytes))
    }

    fn level(&mut self) -> Result<MemoryLevel, LinkError> {
        self.exchange(None)
    }

    // A jail whose leaf cannot be found or written is refused a plug rather than grown past the
    // limit the kernel would enforce by killing it.
    fn set_memory_max(&mut self, bytes: u64) -> Result<(), ()> {
        let leaf = self.leaf.as_ref().ok_or(())?;
        leaf.set_memory_max(bytes).map(|_| ()).map_err(|_| ())
    }
}

/// The leaf a jail for `instance` was built under, from the configured delegated root.
fn jail_leaf(instance: &InstanceId) -> Option<CgroupLeaf> {
    let root = std::env::var_os(super::CGROUP_ROOT)?;
    CgroupLeaf::open_existing(
        std::path::Path::new(&root),
        &format!("soma-{}", instance.as_str()),
    )
    .ok()
}
