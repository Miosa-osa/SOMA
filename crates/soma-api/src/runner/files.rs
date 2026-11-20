//! The platform's sandbox file contract, served for the sandboxes this runner holds.
//!
//! A sandbox the runner created answers the same file calls a sandbox on the platform answers:
//! the same routes under `fs` and `files`, the same parameters, the same bodies, and the
//! platform's own refusal codes. Underneath, each call is one or a few of the guest's bounded
//! file operations (ADR 0051), reached through the facade exactly as soma-api's own loopback
//! filesystem routes reach them.
//!
//! The pieces are kept apart by what they decide: which call a request names (`call`), which
//! guest paths it may reach (`path`), what its parameters say (`params`, `mode`, `plan`), which
//! guest operations perform it (`guest`), and what it answers (`run`, `answer`, `error`).

pub(crate) mod answer;
pub(crate) mod call;
pub(crate) mod error;
pub(crate) mod guest;
pub(crate) mod mode;
pub(crate) mod multipart;
pub(crate) mod params;
pub(crate) mod path;
pub(crate) mod plan;
pub(crate) mod run;
pub(crate) mod upload;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use call::FileCall;
pub(crate) use error::FileError;
pub(crate) use guest::{OnFacade, Stop};
pub(crate) use params::{Params, ParamsRefused};
pub(crate) use plan::{MAX_JSON_FILE_BYTES, Plan};
pub(crate) use run::{Answer, refusal};
pub(crate) use upload::MAX_UPLOAD_BODY_BYTES;

/// The largest body a JSON write may carry: the largest file it moves, as base64, and room for
/// the path and the JSON around it.
pub(crate) const MAX_JSON_WRITE_BODY_BYTES: usize = MAX_JSON_FILE_BYTES.div_ceil(3) * 4 + 64 * 1024;
