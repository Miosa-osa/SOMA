use serde::Serialize;

/// Requested guest attachment policy for Apple Container.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// Leave network selection to the runtime and observe the effective attachment.
    Unspecified,
    /// Require the runtime's no-network mode.
    Denied,
    /// Require attachment to the runtime's built-in default network.
    Allowed,
}
