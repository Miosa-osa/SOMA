//! Which route a request names.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route<'a> {
    Health,
    Create,
    Exec(&'a str),
    Destroy(&'a str),
    NotFound,
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
    match (method, segments.as_slice()) {
        (&http::Method::POST, [""]) => Route::Create,
        (&http::Method::POST, ["", id, "exec"]) if !id.is_empty() => Route::Exec(id),
        (&http::Method::DELETE, ["", id]) if !id.is_empty() => Route::Destroy(id),
        _ => Route::NotFound,
    }
}
