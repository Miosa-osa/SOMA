use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressPolicy {
    /// Leave outbound connectivity to the backend and require an honest observation.
    Unspecified,
    /// Deny every guest-initiated network flow.
    Denied,
    /// Allow public unicast destinations while denying host, tenant, private, link-local, and
    /// infrastructure destinations.
    PublicInternet,
    /// Allow the backend's unfiltered network path.
    ///
    /// This is intended only for explicitly trusted workloads and development environments.
    Unrestricted,
}
