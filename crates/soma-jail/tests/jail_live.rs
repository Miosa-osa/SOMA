//! Privileged live acceptance tests for the VMM jail.
//!
//! Every test is `#[ignore]` and fails explicitly when its prerequisites are missing; nothing
//! passes silently.
//! `scripts/jail-live-tests.sh` builds the static probe and this binary, then runs them as
//! root inside a privileged Ubuntu 24.04 container with a delegated cgroup2 subtree.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "jail_live/containment.rs"]
mod containment;
#[path = "jail_live/control.rs"]
mod control;
#[path = "jail_live/failure.rs"]
mod failure;
#[path = "jail_live/harness.rs"]
mod harness;
#[path = "jail_live/worker.rs"]
mod worker;
