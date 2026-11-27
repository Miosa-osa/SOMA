//! The API version: what a client of this service's `/v1` HTTP surface negotiates.
//!
//! The major names the surface itself: it is the `v1` of every path and of the `soma.api.v1`
//! envelope, and a change no older client can survive needs a new major beside the old one. The
//! minor counts additive revisions of that surface, so a client that needs a route added in
//! `1.2` can say so and be refused cleanly by a `1.1` service instead of meeting a 404.
//! `docs/product/reference/api-surface.md` records the surface of every minor, and a test fails when the
//! surface changes without it.

use std::fmt;

use serde::Serialize;
use soma_version::{Build, Protocol};

use crate::{
    envelope::{ApiError, Envelope, failure_body, render},
    http::{request::Request, response::Response},
    runner::control_plane::RUNNER_PROTOCOL_VERSION,
    tenant::{TENANT_HEADER, identify},
};

/// The API version this build serves.
pub const API_VERSION: ApiVersion = ApiVersion::new(1, 10);

/// The header that carries an API version: the one a response was served under, and the one a
/// request needs, `MAJOR` or `MAJOR.MINOR`.
pub const API_VERSION_HEADER: &str = "soma-api-version";

/// The operation name of `GET /v1/version`.
pub const VERSION_OPERATION: &str = "service.version";

/// One API version: a major that names the surface and a minor that counts additions to it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ApiVersion {
    major: u16,
    minor: u16,
}

impl ApiVersion {
    /// Names one API version.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// The major version, the `v1` of every path.
    #[must_use]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// The minor version, how many additive revisions the major has had.
    #[must_use]
    pub const fn minor(self) -> u16 {
        self.minor
    }

    /// Reads `MAJOR` or `MAJOR.MINOR`; a bare major needs nothing beyond `MAJOR.0`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let number = |part: &str| {
            (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        };
        match text.split_once('.') {
            None => Some(Self::new(number(text)?, 0)),
            Some((major, minor)) => Some(Self::new(number(major)?, number(minor)?)),
        }
    }

    /// Whether a service at this version serves a client that needs `required`.
    ///
    /// It must be the same major, because a major is a different surface, and at least the
    /// required minor, because a minor only ever adds to the surface.
    #[must_use]
    pub const fn serves(self, required: Self) -> bool {
        self.major == required.major && self.minor >= required.minor
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

/// Every protocol `soma-api` speaks: the local runtime's, then the runner's to the control plane.
#[must_use]
pub fn protocols() -> Vec<Protocol> {
    let mut protocols = soma_local::PROTOCOLS.to_vec();
    protocols.push(Protocol::new("runner", u32::from(RUNNER_PROTOCOL_VERSION)));
    protocols
}

/// The report `soma-api --version` prints, for the identity the binary was compiled with.
///
/// The binary expands `soma_version::build!()` and passes it in, so only the binary's own crate
/// reads the commit.
#[must_use]
pub fn version_report(build: &Build) -> String {
    let api = API_VERSION.to_string();
    let protocols = protocols();
    let speaks = soma_version::Speaks {
        api: Some(&api),
        protocols: &protocols,
    };
    soma_version::report("soma-api", build, &speaks)
}

/// The refusal for an identified request that needs an API version this build does not serve.
///
/// An unidentified request is left to the handler, which refuses it as unidentified first: a
/// caller with no identity learns nothing about the surface, its version included.
#[must_use]
pub fn negotiate(request: &Request) -> Option<Response> {
    let stated = request.header(API_VERSION_HEADER)?;
    identify(request.header(TENANT_HEADER)).ok()?;
    let error = match ApiVersion::parse(stated.trim()) {
        None => ApiError::new(
            400,
            "invalid_api_version",
            "the soma-api-version header must be MAJOR or MAJOR.MINOR",
            false,
        ),
        Some(required) if API_VERSION.serves(required) => return None,
        Some(_) => ApiError::new(
            400,
            "api_version_unsupported",
            "this service does not serve the API version the request needs; every response \
             names the version it serves in soma-api-version",
            false,
        ),
    };
    Some(Response::new(
        error.status(),
        failure_body("request", error),
    ))
}

/// Serves `GET /v1/version`: the API version, the build identity, and the protocols spoken.
///
/// Identity comes first, as on every route, so an unidentified caller learns nothing of the
/// build. It needs no sandbox facade, so a burst holding every facade cannot hide the version.
#[must_use]
pub fn handle_version(request: &Request, build: &Build) -> Response {
    if let Err(error) = identify(request.header(TENANT_HEADER)) {
        return Response::new(error.status(), failure_body("request", error));
    }
    let protocols = protocols();
    let body = VersionBody {
        api_version: API_VERSION.to_string(),
        release: build.release(),
        commit: build.commit(),
        commit_short: build.commit_short(),
        commit_date: build.commit_date(),
        source: build.source().as_str(),
        profile: build.profile(),
        target: build.target(),
        protocols: ProtocolMap(&protocols),
    };
    Response::new(
        200,
        render(&Envelope::success(VERSION_OPERATION, &body, None)),
    )
}

#[derive(Serialize)]
struct VersionBody<'a> {
    api_version: String,
    release: &'static str,
    commit: &'static str,
    commit_short: &'static str,
    commit_date: &'static str,
    source: &'static str,
    profile: &'static str,
    target: &'static str,
    protocols: ProtocolMap<'a>,
}

/// The protocols, rendered as one object from name to version.
struct ProtocolMap<'a>(&'a [Protocol]);

impl Serialize for ProtocolMap<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for protocol in self.0 {
            map.serialize_entry(protocol.name, &protocol.version)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests;
