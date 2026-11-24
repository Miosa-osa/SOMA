//! Typed computational kernels for SOMA hot paths.
//!
//! Each kernel is a pure function with a reference implementation kept beside it, so a candidate
//! is admitted only when it returns exactly what the reference returns for every input in its
//! domain (`tests/`) and is measurably faster in its caller (`benches/`, and the ledger in
//! `benchmarks/kernels/candidates.toml`). No kernel validates caller input: validation stays at
//! the trust boundary that already owns it.

pub mod ring;
