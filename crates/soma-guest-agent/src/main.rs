//! Composition root of the SOMA Linux guest agent.
//!
//! The binary is `/init` of the deterministic initramfs and stays PID 1 for the life of the
//! machine.
//! It is gated to Linux `x86_64`, the one target whose kernel interface request layouts the
//! repair modules encode; on every other target the agent builds but refuses to run, and
//! `network_repair::target` refuses again if that gate is ever widened without verified
//! layouts.
//! Invoked with the reserved readiness argument it exits immediately with no output, which is
//! the fixed version 1 self-probe executed through the production executor.

#![cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]

mod environment;
mod output;
mod repair;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod boot;
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
mod identity;
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
mod shutdown;

const PROBE_ARGUMENT: &str = "--soma-ready-probe-v1";

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == PROBE_ARGUMENT)
    {
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
mod agent {
    use std::time::{Duration, Instant};

    use soma_guest::GuestControl;

    use crate::repair::{Controller, Fault, Poisoned, State, Step};
    use crate::{
        boot, console, control, entropy, identity, launch_page, lifecycle, network_repair, pid1,
    };

    /// Console line the agent prints once it is parked at the disconnected repair point.
    ///
    /// A snapshot builder waits for this exact line before it quiesces and captures.
    pub const REPAIR_POINT_LINE: &str = "awaiting launch material";

    const PAGE_POLL: Duration = Duration::from_millis(2);
    const ENTROPY_BUDGET: Duration = Duration::from_secs(5);
    const TRANSPORT_BUDGET: Duration = Duration::from_secs(10);
    const HANDSHAKE_BUDGET: Duration = Duration::from_secs(10);

    pub fn run() -> ! {
        pid1::install_panic_hook();
        if !pid1::is_pid1() {
            console::report("refusing to run outside PID 1");
            pid1::poweroff();
        }
        let controller = Controller::captured();
        if let Err(failure) = boot::early_init(Instant::now() + boot::BOOT_BUDGET) {
            console::report(&format!(
                "boot failed at {:?} errno {}",
                failure.step, failure.errno
            ));
            destroy(&controller.poison(Fault::Boot));
        }
        // The disconnected repair point. Everything the Generation needs on disk is already
        // written, so the agent flushes the private overlay and announces the wait before it
        // blocks: a Generation builder captures the machine exactly here, with no launch
        // material, no session, and no Instance identity anywhere in guest memory.
        pid1::sync();
        console::report(REPAIR_POINT_LINE);
        let (controller, material) = advance(controller.accept_material(
            launch_page::await_and_consume(PAGE_POLL).map_err(|_| Fault::LaunchPage),
        ));
        let binding = *material.binding();
        let network = material.network();
        let hostname = identity::hostname(binding.instance());
        let entropy_deadline = Instant::now() + ENTROPY_BUDGET;
        let (controller, session) = advance(
            controller.repair_entropy(
                material
                    .reseed_with(|seed| entropy::repair(seed, entropy_deadline))
                    .map_err(|_| Fault::Entropy),
            ),
        );
        let (controller, transport) = advance(
            controller.freshen_transport(
                control::connect_vsock(network.vsock_cid(), Instant::now() + TRANSPORT_BUDGET)
                    .map_err(|_| Fault::Transport),
            ),
        );
        let (controller, ()) = advance(
            controller.repair_identity(
                identity::repair(binding.instance(), network.time_sample_nanos())
                    .map_err(|_| Fault::Identity),
            ),
        );
        let (controller, ()) = advance(controller.repair_network(
            network_repair::repair(&network, &hostname).map_err(|_| Fault::Network),
        ));
        let (controller, authenticated) = advance(
            controller.authenticate(
                GuestControl::connect(session, transport, Instant::now() + HANDSHAKE_BUDGET)
                    .map_err(|_| Fault::Control),
            ),
        );
        let (controller, probed) = advance(controller.probe(lifecycle::probe(authenticated)));
        let (controller, ()) = advance(controller.ready(Ok(())));
        console::report("ready");
        destroy(&lifecycle::serve(controller, probed))
    }

    fn advance<S: State, T>(step: Step<S, T>) -> (Controller<S>, T) {
        match step {
            Ok(next) => next,
            Err(poisoned) => destroy(&poisoned),
        }
    }

    fn destroy(poisoned: &Controller<Poisoned>) -> ! {
        console::report(&format!(
            "poisoned by {:?} after {:?}",
            poisoned.fault(),
            poisoned.ledger().repair_chain()
        ));
        pid1::poweroff()
    }
}
