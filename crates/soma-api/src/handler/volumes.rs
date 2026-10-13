//! The durable data volume routes of ADR 0053 Step A.
//!
//! The routes are thin on purpose: the volume store lives in the host daemon, so a handler
//! parses the request, hands it to the facade, and renders what came back. A backend with no
//! host control socket produces the missing capability, and this layer reports it as it came
//! rather than inventing an answer.

use crate::{
    envelope::ApiError,
    facade::SandboxFacade,
    http::response::Response,
    volumes::{VolumeBody, VolumeFailure, VolumeOperation},
};

use super::success;

/// Serves one volume operation.
///
/// # Errors
///
/// Returns the typed refusal for a malformed body, a path that names no volume, or the failure
/// the facade reported.
pub(super) fn serve<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    operation: VolumeOperation,
    path_volume: Option<String>,
    body: &[u8],
) -> Result<Response, ApiError> {
    let request = parse(operation, path_volume, body)?;
    let report = facade
        .volume(operation, &request)
        .map_err(VolumeFailure::refusal)?;
    Ok(success(200, operation.name(), &report, None))
}

/// Parses one volume request; a read takes its identity from the path and carries no body.
fn parse(
    operation: VolumeOperation,
    path_volume: Option<String>,
    body: &[u8],
) -> Result<VolumeBody, ApiError> {
    if operation.has_body() {
        return serde_json::from_slice(body).map_err(|_| malformed());
    }
    Ok(VolumeBody {
        operation_id: String::new(),
        instance_id: String::new(),
        volume_id: path_volume.ok_or_else(ApiError::not_found)?,
        request_fingerprint: String::new(),
        size_class_gib: 0,
        generation_id: None,
        tenant: None,
    })
}

fn malformed() -> ApiError {
    ApiError::new(
        400,
        "invalid_volume_request",
        "the volume request is malformed",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_takes_its_identity_from_the_path() {
        let body = parse(VolumeOperation::Read, Some("ab".to_owned()), b"").expect("parses");
        assert_eq!(body.volume_id, "ab");
        assert!(parse(VolumeOperation::Read, None, b"").is_err());
    }

    #[test]
    fn a_body_operation_refuses_a_body_it_cannot_read() {
        assert!(parse(VolumeOperation::Detach, None, b"not json").is_err());
        let body = parse(
            VolumeOperation::Detach,
            None,
            br#"{"operation_id":"a1","instance_id":"b2","volume_id":"c3","request_fingerprint":"d4"}"#,
        )
        .expect("parses");
        assert_eq!(body.volume_id, "c3");
        assert_eq!(body.size_class_gib, 0);
    }
}
