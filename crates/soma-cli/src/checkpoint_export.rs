use crate::{cli::Cli, exit::ProcessExit};

pub fn execute(cli: &Cli) -> ProcessExit {
    execute_on_host(cli).unwrap_or_else(|exit| {
        let _ignored = super::eprintln_bounded(message(exit));
        exit
    })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn execute_on_host(cli: &Cli) -> Result<ProcessExit, ProcessExit> {
    use std::io::{self, Read as _, Write as _};

    use soma_local::{
        CheckpointArtifact, CheckpointFailure, CheckpointReceipt, LocalRuntime, LocalRuntimeConfig,
    };

    use crate::cli::{ExportArtifact, RootCommand};

    const MAX_RECEIPT_BYTES: u64 = 64 * 1024;

    let RootCommand::CheckpointExport(arguments) = &cli.command else {
        return Err(ProcessExit::Usage);
    };
    let Some(state_root) = cli.state_root.as_ref() else {
        return Err(ProcessExit::Usage);
    };

    let owner =
        soma::MachineName::parse(&arguments.owner).map_err(|_| ProcessExit::InvalidInput)?;
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProcessExit::InvalidInput)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(ProcessExit::InvalidInput);
    }
    let receipt: CheckpointReceipt =
        serde_json::from_slice(&bytes).map_err(|_| ProcessExit::InvalidInput)?;
    let config = LocalRuntimeConfig::discover(
        cli.backend.into(),
        cli.runtime.clone(),
        Some(state_root.clone()),
    )
    .map_err(|_| ProcessExit::InvalidInput)?
    .with_hosted_machines(true);
    let mut runtime = LocalRuntime::open(config).map_err(|_| ProcessExit::BackendFailure)?;
    let mut export =
        runtime
            .open_checkpoint_export(&owner, &receipt)
            .map_err(|error| match error {
                CheckpointFailure::NotFound => ProcessExit::NotFound,
                CheckpointFailure::CaptureRefused => ProcessExit::Conflict,
                CheckpointFailure::Unsupported => ProcessExit::CapabilityUnavailable,
                CheckpointFailure::Unconfirmed => ProcessExit::CleanupUncertain,
                CheckpointFailure::Storage => ProcessExit::BackendFailure,
            })?;
    let artifact = match arguments.artifact {
        ExportArtifact::State => CheckpointArtifact::State,
        ExportArtifact::Memory => CheckpointArtifact::Memory,
        ExportArtifact::Overlay => CheckpointArtifact::Overlay,
        ExportArtifact::Workspace => CheckpointArtifact::Workspace,
    };
    let mut output = io::stdout().lock();
    if export
        .write_artifact(artifact, &mut output)
        .map_err(|_| ProcessExit::BackendFailure)?
        .is_none()
    {
        return Err(ProcessExit::NotFound);
    }
    output.flush().map_err(|_| ProcessExit::BackendFailure)?;
    Ok(ProcessExit::Success)
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn execute_on_host(cli: &Cli) -> Result<ProcessExit, ProcessExit> {
    use crate::cli::RootCommand;
    if let RootCommand::CheckpointExport(arguments) = &cli.command {
        let _ = (&arguments.owner, arguments.artifact);
    }
    Err(ProcessExit::CapabilityUnavailable)
}

const fn message(exit: ProcessExit) -> &'static str {
    match exit {
        ProcessExit::Usage => "soma: checkpoint export requires an explicit --state-root",
        ProcessExit::InvalidInput => "soma: invalid checkpoint receipt or owner",
        ProcessExit::NotFound => "soma: checkpoint artifact unavailable for the recorded owner",
        ProcessExit::Conflict => "soma: checkpoint receipt does not match the capture",
        ProcessExit::CapabilityUnavailable => {
            "soma: checkpoint export requires an encrypted KVM host"
        }
        ProcessExit::CleanupUncertain => {
            "soma: checkpoint export could not acquire the restore lease"
        }
        _ => "soma: checkpoint transfer could not be verified",
    }
}
