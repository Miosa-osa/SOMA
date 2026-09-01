//! One KVM sandbox, its authenticated guest session, and the thread that owns both.
//!
//! A running machine cannot be moved between threads: its vCPU holds a process-wide handler
//! guard that is not `Send`, and the guest-control adapter borrows the machine so that
//! committing repair can retire the launch page. A structure holding both would have to refer
//! to itself. So one thread owns the machine for its whole life and everything else speaks to
//! it over channels.
//!
//! That shape is why this lives here rather than beside the host adapters that used to own it.
//! Two processes now drive the same sandbox: the resident one that holds a machine for as long
//! as its own command runs, and the jailed worker that holds one on behalf of a broker outside
//! the jail. Both drive it identically, and a second implementation of a machine's life is the
//! one thing that could make them disagree.
//!
//! Nothing in here resolves a path. Everything a sandbox is built from arrives as an open
//! handle, because the jailed side has an empty root, no procfs, and a filter that kills `open`
//! once it narrows.

mod assign;
mod file;
mod identity;
mod io;
mod operations;
mod pending;
mod secrets;
mod session;
mod source;
mod sterile;
mod timeline;
mod worker;

pub use identity::{
    FIRST_GUEST_CID, GUEST_MAC, fresh16, guest_cid_for, link_down_network, now_unix_nanos,
};
pub use pending::PendingActivation;
pub use session::{BOOT_DEADLINE, Completed, EXIT_GRACE, Session, SessionError};
pub use source::{Boot, Network, Source};
pub use sterile::{Assignment, SterileSpec};
pub use timeline::dump as dump_timeline;
pub use worker::{ColdBootInputs, config as cold_boot_config};
