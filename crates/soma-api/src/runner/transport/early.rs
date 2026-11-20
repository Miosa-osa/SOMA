//! The answers the transport gives before a request reaches the runner: a body that was too
//! large or too slow to arrive. Both are refused before any stage ran, so both carry a
//! `Server-Timing` header of zeroes, as every answer carries the header.

use crate::runner::{public_wire, service::RunnerResponse};

pub(crate) fn request_timeout() -> RunnerResponse {
    let mut response = payload_too_large();
    response.status = 408;
    response.body = public_wire::refusal("request_timeout");
    response
}

pub(crate) fn payload_too_large() -> RunnerResponse {
    RunnerResponse {
        status: 413,
        // Refused before any stage ran, but every answer carries the header all the same.
        headers: vec![(
            "server-timing",
            "auth;dur=0.000,pool;dur=0.000,exec;dur=0.000".to_owned(),
        )],
        body: public_wire::refusal("payload_too_large"),
        stream: None,
        journal: None,
        tunnel: None,
    }
}
