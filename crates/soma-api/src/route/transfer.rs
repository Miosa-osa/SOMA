//! The checkpoint transfer routes: export to, and import from, off-host storage.
//!
//! They exist only on the loopback host listener and answer only the platform tenant
//! (ADR 0070). The caller owns the object store, its names and its credentials; these routes move
//! the exact capture artifacts between that caller and the host's encrypted checkpoint root.

use soma::OperationId;

use crate::{envelope::ApiError, http::request::Method};

/// One artifact of the fixed checkpoint format, as a transfer path names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferArtifact {
    State,
    Memory,
    Overlay,
    Workspace,
}

impl TransferArtifact {
    /// The artifact named on a path or in a request body.
    ///
    /// # Errors
    ///
    /// Returns a 404 refusal for a name the format does not have.
    pub fn parse(name: &str) -> Result<Self, ApiError> {
        match name {
            "state" => Ok(Self::State),
            "memory" => Ok(Self::Memory),
            "overlay" => Ok(Self::Overlay),
            "workspace" => Ok(Self::Workspace),
            _ => Err(ApiError::not_found()),
        }
    }

    /// The artifact's own name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Memory => "memory",
            Self::Overlay => "overlay",
            Self::Workspace => "workspace",
        }
    }
}

/// One resolved checkpoint transfer route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferRoute {
    /// `POST /v1/checkpoints/export`: one artifact of a published capture, streamed raw.
    Export,
    /// `PUT /v1/checkpoints/imports/{import}/{artifact}`: one artifact received raw.
    Receive(OperationId, TransferArtifact),
    /// `POST /v1/checkpoints/imports/{import}/commit`: verify and publish what was received.
    Commit(OperationId),
    /// `DELETE /v1/checkpoints/imports/{import}`: drop what was received.
    Abort(OperationId),
}

impl TransferRoute {
    /// Names the operation for the response envelope.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::Export => "checkpoint.export",
            Self::Receive(..) => "checkpoint.import.receive",
            Self::Commit(_) => "checkpoint.import.commit",
            Self::Abort(_) => "checkpoint.import.abort",
        }
    }
}

/// Resolves the segments after `/v1/checkpoints`.
pub(super) fn resolve(method: Method, rest: &[&str]) -> Result<TransferRoute, ApiError> {
    let route = match rest {
        ["export"] => (Method::Post, TransferRoute::Export),
        ["imports", import, "commit"] => (Method::Post, TransferRoute::Commit(id(import)?)),
        ["imports", import, artifact] => (
            Method::Put,
            TransferRoute::Receive(id(import)?, TransferArtifact::parse(artifact)?),
        ),
        ["imports", import] => (Method::Delete, TransferRoute::Abort(id(import)?)),
        _ => return Err(ApiError::not_found()),
    };
    if route.0 == method {
        Ok(route.1)
    } else {
        Err(ApiError::method_not_allowed())
    }
}

fn id(segment: &str) -> Result<OperationId, ApiError> {
    OperationId::new(segment.to_ascii_lowercase())
        .map_err(|_| ApiError::invalid("the import identity is not a valid operation identity"))
}

#[cfg(test)]
mod tests {
    use super::{Method, TransferArtifact, TransferRoute, resolve};
    use crate::envelope::ApiError;

    #[test]
    fn each_transfer_path_resolves_under_its_one_method() {
        let import = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            resolve(Method::Post, &["export"]),
            Ok(TransferRoute::Export)
        );
        assert!(matches!(
            resolve(Method::Put, &["imports", import, "memory"]),
            Ok(TransferRoute::Receive(_, TransferArtifact::Memory))
        ));
        assert!(matches!(
            resolve(Method::Post, &["imports", import, "commit"]),
            Ok(TransferRoute::Commit(_))
        ));
        assert!(matches!(
            resolve(Method::Delete, &["imports", import]),
            Ok(TransferRoute::Abort(_))
        ));
        assert_eq!(
            resolve(Method::Get, &["export"]).map_err(ApiError::status),
            Err(405)
        );
        assert_eq!(
            resolve(Method::Put, &["imports", import, "receipt"]).map_err(ApiError::status),
            Err(404)
        );
        assert_eq!(
            resolve(Method::Put, &["imports", "not-an-id", "memory"]).map_err(ApiError::status),
            Err(400)
        );
    }
}
