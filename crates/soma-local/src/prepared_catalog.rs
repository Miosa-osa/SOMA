//! What this host's prepared store can launch, read through the lookup a launch uses.
//!
//! Each entry is resolved exactly as a create would resolve it, so the shape reported is the
//! shape the backend would admit against: the boot memory, and the ceiling the Generation's own
//! memory device allows (`fit` and `hotplug::room_for` read the same two numbers). A caller that
//! places work by this list therefore never offers a size the host would then refuse.

use serde::Serialize;

/// One prepared entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PreparedSummary {
    /// The image reference the entry was prepared for.
    pub reference: String,
    /// The Generation a launch of `reference` uses, when the entry resolves.
    pub generation_id: Option<String>,
    /// The vCPUs the Generation booted with: the most a request may ask for.
    pub vcpus: Option<u16>,
    /// The memory the Generation boots with: the least a request may ask for.
    pub memory_mib: Option<u64>,
    /// The most memory the Generation can ever hold: its boot memory plus the region its memory
    /// device declares. Equal to `memory_mib` when it declares none and cannot grow.
    pub ceiling_mib: Option<u64>,
    /// The writable disk every instance of the Generation gets.
    pub disk_mib: Option<u64>,
    /// The durable volume class the Generation declares, when it declares one.
    pub volume_class_gib: Option<u32>,
    /// The compiler policy the Generation was built under.
    pub compiler_policy: Option<u16>,
    /// Why the entry does not resolve, when it does not; every other field is then `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl PreparedSummary {
    pub(crate) const fn refused(reference: String, refusal: &'static str) -> Self {
        Self {
            reference,
            generation_id: None,
            vcpus: None,
            memory_mib: None,
            ceiling_mib: None,
            disk_mib: None,
            volume_class_gib: None,
            compiler_policy: None,
            refusal: Some(refusal),
        }
    }
}

/// Why the catalog cannot be read at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// This host names no prepared store, or this target has none.
    Unsupported,
    /// The named store cannot be read.
    StoreUnavailable,
}

/// Every prepared entry on this host, in reference order.
///
/// # Errors
///
/// Returns [`CatalogError::Unsupported`] when the host prepares nothing and
/// [`CatalogError::StoreUnavailable`] when its store cannot be listed.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn prepared_catalog() -> Result<Vec<PreparedSummary>, CatalogError> {
    crate::backend::prepared_catalog()
}

/// Non-KVM targets have no prepared store.
///
/// # Errors
///
/// Always [`CatalogError::Unsupported`].
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn prepared_catalog() -> Result<Vec<PreparedSummary>, CatalogError> {
    Err(CatalogError::Unsupported)
}
