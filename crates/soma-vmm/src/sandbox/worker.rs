//! The body of the thread that owns one machine.
//!
//! Everything here runs on the sandbox thread, which is why the host adapter may borrow the
//! machine: both are locals of the same stack frame for the machine's whole life.
//!
//! A sandbox arrives at Ready one of two ways. A cold boot builds a machine and runs the kernel
//! and userspace init on the request path. A restore resumes a machine captured once for the
//! whole Generation, already past that work. After Ready the two are identical, so the command
//! loop is written once and both paths enter it.
//!
//! The four jobs are kept apart here. `entry` decides which of the two ways this sandbox takes,
//! `drive` runs the half both of them share, `finish` releases the machine and reports what it
//! left behind, and `inputs` names the shape a cold boot starts from. The device and command
//! work they call sits in the sibling modules below.

mod activation;
mod commands;
mod drive;
mod entry;
mod files;
mod finish;
mod inputs;
mod pty;

pub use drive::drive_restored;
pub use entry::serve;
pub use finish::{Ending, report};
pub use inputs::{ColdBootInputs, LaunchInputs, config};
