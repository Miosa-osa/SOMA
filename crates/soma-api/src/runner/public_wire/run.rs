//! The `run` object a create-and-run answers with.
//!
//! It lives apart from the other bodies because it is the only one the runner adds to a create
//! and the only one the exec door and the create door share.

use serde::Serialize;

use serde_json::Value;

/// The `run` object a create-and-run's `201` carries.
///
/// It is the exec endpoint's own result, moved into the create the caller asked for: a command
/// that ended reports its exit code and its output exactly as `POST .../exec` would have, and a
/// command that produced no result reports exactly the `error` object that endpoint would have
/// answered with, with `exit_code` null to say there was no result of its own.
#[derive(Serialize)]
pub struct Ran<'a> {
    /// How long the command ran, server-side, in whole milliseconds.
    pub duration_ms: u64,
    /// The answer the exec endpoint would have given, when the command produced no result.
    pub error: Option<Value>,
    pub exit_code: Option<i32>,
    pub stderr: &'a str,
    pub stdout: &'a str,
}
