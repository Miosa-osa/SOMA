use super::{MAX_ARTIFACT_BYTES, announced_length, platform_only};
use crate::envelope::ApiError;
use crate::http::request::{Method, Request};

fn request(headers: &[(&str, &str)]) -> Request {
    Request::from_parts(
        Method::Put,
        "/v1/checkpoints/imports/0123456789abcdef0123456789abcdef/memory".to_owned(),
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        Vec::new(),
    )
}

#[test]
fn only_the_platform_reaches_a_transfer_route() {
    assert_eq!(
        platform_only(&request(&[("x-soma-tenant", "soma-platform")])),
        Ok(())
    );
    let tenant = platform_only(&request(&[("x-soma-tenant", "acme")])).expect_err("a tenant");
    assert_eq!(tenant.status(), 404);
    let anonymous = platform_only(&request(&[])).expect_err("no identity");
    assert_eq!(anonymous.status(), 401);
}

#[test]
fn a_received_artifact_needs_an_exact_bounded_length() {
    assert_eq!(
        announced_length(&request(&[("content-length", "4096")])),
        Ok(4096)
    );
    let chunked = announced_length(&request(&[("transfer-encoding", "chunked")]));
    assert_eq!(chunked.map_err(ApiError::status), Err(411));
    assert_eq!(
        announced_length(&request(&[])).map_err(ApiError::status),
        Err(411)
    );
    let too_large = (MAX_ARTIFACT_BYTES + 1).to_string();
    assert_eq!(
        announced_length(&request(&[("content-length", too_large.as_str())]))
            .map_err(ApiError::status),
        Err(413)
    );
    assert_eq!(
        announced_length(&request(&[("content-length", "ten")])).map_err(ApiError::status),
        Err(400)
    );
}
