//! Which route a request names.

use crate::http::request::Method;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route<'a> {
    Health,
    Create,
    Exec(&'a str),
    Destroy(&'a str),
    /// `PATCH {"timeout": N}`: reset or extend the idle timer.
    Extend(&'a str),
    /// The caller's own sandboxes on this runner.
    List,
    Forward(Forward<'a>),
    NotFound,
}

/// Any other per-sandbox route soma-api serves (contract C2), handed to the loopback handler
/// as `method /v1/sandboxes/<instance><suffix>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Forward<'a> {
    pub(super) id: &'a str,
    pub(super) method: Method,
    pub(super) suffix: &'a str,
}

pub(super) fn route<'a>(method: &http::Method, path: &'a str) -> Route<'a> {
    if path == "/healthz" {
        return if method == http::Method::GET {
            Route::Health
        } else {
            Route::NotFound
        };
    }
    let Some(rest) = path.strip_prefix("/api/v1/sandboxes") else {
        return Route::NotFound;
    };
    let segments: Vec<&str> = rest.split('/').collect();
    let forward = |id: &'a str, method: Method| {
        Route::Forward(Forward {
            id,
            method,
            suffix: &rest[1 + id.len()..],
        })
    };
    match (method, segments.as_slice()) {
        (&http::Method::POST, [""]) => Route::Create,
        (&http::Method::GET, [""]) => Route::List,
        (&http::Method::POST, ["", id, "exec"]) if !id.is_empty() => Route::Exec(id),
        (&http::Method::DELETE, ["", id]) if !id.is_empty() => Route::Destroy(id),
        (&http::Method::PATCH, ["", id]) if !id.is_empty() => Route::Extend(id),
        (&http::Method::GET, ["", id]) if !id.is_empty() => forward(id, Method::Get),
        (&http::Method::POST, ["", id, "stop"]) if !id.is_empty() => forward(id, Method::Post),
        (&http::Method::POST, ["", id, "filesystem" | "terminal", operation])
            if !id.is_empty() && !operation.is_empty() =>
        {
            forward(id, Method::Post)
        }
        _ => Route::NotFound,
    }
}
