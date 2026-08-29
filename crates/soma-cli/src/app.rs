mod doctor;
mod failure;
mod operation;
mod success;

use crate::{
    cli::{Cli, RootCommand},
    exit::ProcessExit,
    model::{CapabilityState, ENVELOPE_SCHEMA, Response, ResultBody, VersionReport},
    request::{prepare_machine, prepare_run},
};

use self::{doctor::doctor, failure::invalid, operation::invoke, success::success};

pub struct Execution {
    pub response: Response,
    pub exit: ProcessExit,
}

pub fn execute(cli: Cli) -> Execution {
    let Cli {
        backend,
        runtime,
        state_root,
        command,
        ..
    } = cli;
    match command {
        RootCommand::Version => version(),
        RootCommand::Doctor(arguments) => doctor(backend, runtime, arguments.strict),
        RootCommand::Run(arguments) => prepare_run(arguments).map_or_else(
            |error| invalid("run", error),
            |operation| invoke(backend, runtime, state_root, operation),
        ),
        RootCommand::Machine(arguments) => prepare_machine(arguments).map_or_else(
            |error| invalid("machine", error),
            |operation| invoke(backend, runtime, state_root, operation),
        ),
    }
}

fn version() -> Execution {
    success(
        "version",
        ResultBody::Version(VersionReport {
            version: env!("CARGO_PKG_VERSION"),
            envelope_schema: ENVELOPE_SCHEMA,
            production_ready: false,
            macos_development_lifecycle: capability(cfg!(all(
                target_os = "macos",
                target_arch = "aarch64"
            ))),
            native_kvm_lifecycle: CapabilityState::Unavailable,
        }),
    )
}

const fn capability(compiled: bool) -> CapabilityState {
    if compiled {
        CapabilityState::Compiled
    } else {
        CapabilityState::Unavailable
    }
}
