//! What a create's `network` object asks for, and how it shapes the machine.
//!
//! It lives apart from the rest of the create vocabulary so the parameter module stays inside the
//! source budget; a child module may read the parent's imports, so the split costs nothing but a
//! `use`.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

/// What a create's `network.egress` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runner::service) enum EgressRequest {
    /// No TCP, no UDP, no DNS.
    Denied,
    /// Public destinations only, with the system resolvers.
    Internet,
    /// The operator-admitted unfiltered network.
    Unrestricted,
}

impl EgressRequest {
    /// The word the create body used and its answer echoes.
    pub(in crate::runner::service) const fn word(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Internet => "internet",
            Self::Unrestricted => "unrestricted",
        }
    }

    /// Whether serving it needs the host's network broker. A windowed `denied` does too, which
    /// the caller adds.
    pub(in crate::runner::service) const fn needs_broker(self) -> bool {
        !matches!(self, Self::Denied)
    }

    fn capabilities(self) -> Capabilities {
        match self {
            Self::Denied => Capabilities::isolated(),
            Self::Internet => Capabilities::isolated().with_network_access(),
            Self::Unrestricted => Capabilities::isolated().with_unrestricted_network(),
        }
    }
}

/// Applies a create's `network` object to the shape it selected.
///
/// The object has one key, `egress`, and an unknown key is refused so a misspelling is never
/// served as the default network. A create that names no network keeps the host's own default,
/// which is how every create behaved before this field existed.
pub(super) fn apply_network(
    shape: MachineShape,
    network: Option<&Value>,
    launch: &LaunchConfig,
) -> Result<(MachineShape, Option<EgressRequest>, bool), PlatformError> {
    let invalid = |detail: &str| PlatformError::invalid_param("network", detail);
    let object = match network {
        None | Some(Value::Null) => return Ok((shape, None, false)),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(invalid("network must be an object")),
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "egress" | "windowed"))
    {
        return Err(invalid(&format!("network has no `{unknown}` field")));
    }
    let egress = match object.get("egress").map(Value::as_str) {
        Some(Some("denied")) => EgressRequest::Denied,
        Some(Some("internet")) => EgressRequest::Internet,
        Some(Some("unrestricted")) => EgressRequest::Unrestricted,
        _ => {
            return Err(invalid(
                "network.egress must be \"denied\", \"internet\" or \"unrestricted\"",
            ));
        }
    };
    let windowed = match object.get("windowed") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(windowed)) => *windowed,
        Some(_) => return Err(invalid("network.windowed must be a boolean")),
    };
    // A window opens the internet on a sandbox that starts with none, so it only makes sense
    // there; asking for one next to an already open network is a mistake worth refusing.
    if windowed && egress != EgressRequest::Denied {
        return Err(invalid("network.windowed needs network.egress \"denied\""));
    }
    if egress == EgressRequest::Unrestricted && !launch.allow_unrestricted_egress {
        return Err(PlatformError::new(
            403,
            "egress_not_permitted",
            "this runner does not serve unrestricted egress",
            false,
        ));
    }
    Ok((
        shape.with_capabilities(egress.capabilities()),
        Some(egress),
        windowed,
    ))
}
