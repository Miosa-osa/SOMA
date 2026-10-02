//! The public runner end to end: real TLS, HTTP/2, HTTP/1.1, and HTTP/3 on a loopback port,
//! a fake control plane on mTLS serving the key feed and taking the journal, and a fake facade in
//! place of KVM.
//!
//! The certificates under `tests/fixtures/runner` are throwaway test material issued by a test
//! CA whose key was discarded; they authenticate nothing outside these tests.

mod clients;
mod control_plane;
mod facade;
mod forwarding;
mod lifecycle;
mod stale;
mod support;
mod tenancy;
