mod doctor;
mod failure;
mod operation;
mod success;

use crate::{
    cli::{Cli, RootCommand},
    exit::ProcessExit,
    model::{
        CapabilityState, ENVELOPE_SCHEMA, ProtocolVersions, Response, ResultBody, VersionReport,
    },
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
        // The binary takes this over before any response is built, so reaching it here means it
        // was reached some other way, and there is no envelope that describes serving a machine.
        RootCommand::MachineHost(_) => Execution {
            response: Response::failure("machine-host", crate::model::FailureBody::usage()),
            exit: ProcessExit::Usage,
        },
        RootCommand::CheckpointExport(_) => Execution {
            response: Response::failure("checkpoint-export", crate::model::FailureBody::usage()),
            exit: ProcessExit::Usage,
        },
    }
}

fn version() -> Execution {
    let build = soma_version::build!();
    success(
        "version",
        ResultBody::Version(VersionReport {
            version: build.release(),
            commit: build.commit(),
            commit_date: build.commit_date(),
            source: build.source().as_str(),
            profile: build.profile(),
            target: build.target(),
            protocols: ProtocolVersions(soma_local::PROTOCOLS),
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
