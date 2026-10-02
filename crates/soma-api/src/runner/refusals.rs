//! The runner's own short bodies (contract C2), beside the fast-lane shapes in `public_wire`.

use serde::Serialize;

use crate::runner::public_wire::encode;

/// A runner refusal from contract C2: `{"error":"<code>"}`.
#[must_use]
pub fn refusal(code: &str) -> Vec<u8> {
    encode(&serde_json::json!({ "error": code }))
}

/// The `421` body naming the runner that owns the sandbox.
#[must_use]
pub fn misdirected(runner_url: &str) -> Vec<u8> {
    #[derive(Serialize)]
    struct Misdirected<'a> {
        error: &'a str,
        runner_url: &'a str,
    }
    encode(&Misdirected {
        error: "misdirected",
        runner_url,
    })
}

/// The `/healthz` body, in the field order contract C2 writes it.
#[must_use]
pub fn health(tag: char, feed_age_ms: u64, pool_ready: Option<u64>) -> Vec<u8> {
    #[derive(Serialize)]
    struct Health {
        ok: bool,
        tag: char,
        feed_age_ms: u64,
        pool_ready: Option<u64>,
    }
    encode(&Health {
        ok: true,
        tag,
        feed_age_ms,
        pool_ready,
    })
}
