//! Composition root of the SOMA Linux guest agent.
//!
//! The binary is `/init` of the deterministic initramfs and stays PID 1 for the life of the
//! machine.
//! It is gated to Linux `x86_64`, the one target whose kernel interface request layouts the
//! repair modules encode; on every other target the agent builds but refuses to run, and
//! `network_repair::target` refuses again if that gate is ever widened without verified
//! layouts.

#![cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod environment;
mod output;
mod repair;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod boot;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod capture_warm;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod console;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod control;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod descendants;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod entropy;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod executor;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod filesystem;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod hotplug;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod identity;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod idle_reaper;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod ioctl;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod launch_page;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod lifecycle;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod mounts;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod network_repair;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod pid1;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod priority;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod pty;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod shutdown;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod sizing;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod survivors;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod timings;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod tuning;
mod version;

#[cfg(target_os = "linux")]
mod warm;

fn main() {
    // Answered before anything else, so a release packager can read a built agent's contracts
    // on the build host. As PID 1 the agent is started by the kernel, which never passes it this.
    if soma_version::version_requested(std::env::args_os()) {
        print!("{}", version::report());
        return;
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    agent::run();
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        eprintln!("soma-guest-agent runs only as Linux x86_64 PID 1");
        std::process::exit(2);
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod agent;
