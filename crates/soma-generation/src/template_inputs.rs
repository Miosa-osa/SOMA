//! The external inputs Template resolution needs, answered from what the build already has.
//!
//! Resolving a Template document into a Template Lock asks two questions this crate can answer
//! and `soma-template` deliberately cannot: which exact OCI manifest the workload reference
//! names, and whether the base image really carries the command's program. Until this module
//! existed both questions had only test stubs, so a Template Lock could be produced in a test
//! and nowhere else, and nothing turned one into a Generation.
//!
//! [`LayoutResolver`] answers the first from the local OCI layout the build already exported,
//! [`RootfsOracle`] answers the second from the normalized rootfs the compiler already builds,
//! and [`compiler_revision`] projects the resulting Lock onto the compiler's input contract.
//! Together they close the path from a document on disk to a prepared Generation Candidate.
//!
//! # What a Lock carries that the compiler has nowhere to put
//!
//! The projection is deliberately lossy, and the losses are named here rather than hidden. A
//! Generation binds an image, a Machine shape, a startup behavior, a lifetime, and a profile
//! version; a Lock binds considerably more. These locked fields reach nothing today:
//!
//! - `command`, meaning the program, arguments, working directory, and user. The compiler has
//!   no command field at all: the readiness command is fixed and the workload command arrives
//!   at Launch. The locked program still does work here, because [`RootfsOracle`] proves it
//!   exists in the base image before the Lock is issued.
//! - `modules`. Staging module content into the normalized root is ticket T7 and the compiler
//!   has no input for it, so a Lock that composes modules compiles to the same Generation as
//!   one that composes none. Module health probes would be the natural source of an explicit
//!   workload probe, which is therefore left at readiness only.
//! - `environment` and `secrets`. Both are Launch-time delivery, and neither the Candidate nor
//!   the Generation manifest has a slot for them.
//! - `lifecycle.idle_timeout_seconds` and `lifecycle.on_idle`. Only the maximum lifetime
//!   projects, as the compiler's `LifetimeLimits`; the idle policy belongs to the Backend.
//! - `network` with allowlist egress or with unrestricted ingress. The portable `NetworkPolicy`
//!   has no destination-filtered egress class, so the projection fails closed rather than
//!   widening or silently narrowing the envelope. `templates/coding-agent.toml` is this case.
//! - `ceiling`, `backend`, `content_digest`, `lock_id`, and `policy_version`. These are the
//!   provenance of the decision, and certification does not bind any of them yet.

mod lookup;
mod oracle;
mod resolver;
mod revision;

pub use oracle::RootfsOracle;
pub use resolver::LayoutResolver;
pub use revision::{LockProjectionError, compiler_revision, profile_v1_backend};
