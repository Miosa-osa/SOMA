//! Records the profile and target this crate is compiled for.
//!
//! Both are fixed for one target directory, so this script runs once and never again: it reads
//! no repository state and no clock. The source commit is not read here on purpose. A build
//! script that followed the commit would rerun on every commit, and every crate that depends on
//! this one would recompile with it, the network and host daemons included. The commit reaches
//! a binary through [`build!`](src/lib.rs) instead, which only the binary's own crate expands.

use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    for (name, source) in [
        ("SOMA_VERSION_PROFILE", "PROFILE"),
        ("SOMA_VERSION_TARGET", "TARGET"),
    ] {
        let value = env::var(source).unwrap_or_else(|_| "unknown".to_owned());
        println!("cargo::rustc-env={name}={value}");
    }
}
