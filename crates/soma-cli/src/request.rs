use soma::{
    DestroyMachineRequest, DirectCommand, ExecuteMachineRequest, ExecutionLimits,
    FileMachineRequest, InspectMachineRequest, InstanceId, LaunchMachineRequest, MachineName,
    MachineShape, OciImage, OperationId, RunRequest, StopMachineRequest,
};
use uuid::Uuid;

use crate::cli::{
    ControlArgs, ExecArgs, IdentityArgs, LaunchArgs, MachineArgs, MachineCommand, RunArgs,
    ShapeArgs,
};

mod file;
mod network;

pub enum PreparedOperation {
    Run {
        instance_id: InstanceId,
        request: RunRequest,
    },
    Launch {
        instance_id: InstanceId,
        request: LaunchMachineRequest,
    },
    Execute {
        instance_id: InstanceId,
        request: ExecuteMachineRequest,
    },
    Inspect {
        instance_id: InstanceId,
        request: InspectMachineRequest,
    },
    Stop {
        instance_id: InstanceId,
        request: StopMachineRequest,
    },
    Destroy {
        instance_id: InstanceId,
        request: DestroyMachineRequest,
    },
    File {
        instance_id: InstanceId,
        request: FileMachineRequest,
    },
}

impl PreparedOperation {
    #[must_use]
    pub const fn command(&self) -> &'static str {
        match self {
            Self::Run { .. } => "run",
            Self::Launch { .. } => "machine.launch",
            Self::Execute { .. } => "machine.exec",
            Self::Inspect { .. } => "machine.inspect",
            Self::Stop { .. } => "machine.stop",
            Self::Destroy { .. } => "machine.destroy",
            Self::File { .. } => "machine.file",
        }
    }
}

pub fn prepare_run(arguments: RunArgs) -> Result<PreparedOperation, RequestError> {
    let (operation_id, instance_id) = identities(arguments.identity)?;
    let shape = shape(arguments.shape)?;
    let image = OciImage::parse(arguments.image).map_err(|_| RequestError::Image)?;
    let command = command(arguments.command)?;
    let limits = limits(
        arguments.limits.timeout_ms,
        arguments.limits.max_output_bytes,
    )?;
    let mut request = RunRequest::new(
        operation_id.clone(),
        instance_id.clone(),
        image,
        shape,
        command,
        limits,
    );
    if let Some(name) = arguments.machine_name {
        request = request.with_name(MachineName::parse(name).map_err(|_| RequestError::Name)?);
    }
    Ok(PreparedOperation::Run {
        instance_id,
        request,
    })
}

pub fn prepare_machine(arguments: MachineArgs) -> Result<PreparedOperation, RequestError> {
    match arguments.command {
        MachineCommand::Launch(arguments) => prepare_launch(arguments),
        MachineCommand::Exec(arguments) => prepare_execute(arguments),
        MachineCommand::Inspect(arguments) => prepare_control(arguments, ControlKind::Inspect),
        MachineCommand::Stop(arguments) => prepare_control(arguments, ControlKind::Stop),
        MachineCommand::Destroy(arguments) => prepare_control(arguments, ControlKind::Destroy),
        MachineCommand::File(arguments) => file::prepare(arguments),
    }
}

fn prepare_launch(arguments: LaunchArgs) -> Result<PreparedOperation, RequestError> {
    let (operation_id, instance_id) = identities(arguments.identity)?;
    let mut request = LaunchMachineRequest::new(
        operation_id.clone(),
        instance_id.clone(),
        OciImage::parse(arguments.image).map_err(|_| RequestError::Image)?,
        shape(arguments.shape)?,
    );
    if let Some(name) = arguments.machine_name {
        request = request.with_name(MachineName::parse(name).map_err(|_| RequestError::Name)?);
    }
    Ok(PreparedOperation::Launch {
        instance_id,
        request,
    })
}

fn prepare_execute(arguments: ExecArgs) -> Result<PreparedOperation, RequestError> {
    let operation_id = operation_id(arguments.operation_id)?;
    let instance_id = InstanceId::new(arguments.instance_id).map_err(|_| RequestError::Identity)?;
    let request = ExecuteMachineRequest::new(
        operation_id.clone(),
        instance_id.clone(),
        command(arguments.command)?,
        limits(
            arguments.limits.timeout_ms,
            arguments.limits.max_output_bytes,
        )?,
    );
    Ok(PreparedOperation::Execute {
        instance_id,
        request,
    })
}

#[derive(Clone, Copy)]
enum ControlKind {
    Inspect,
    Stop,
    Destroy,
}

fn prepare_control(
    arguments: ControlArgs,
    kind: ControlKind,
) -> Result<PreparedOperation, RequestError> {
    let operation_id = operation_id(arguments.operation_id)?;
    let instance_id = InstanceId::new(arguments.instance_id).map_err(|_| RequestError::Identity)?;
    Ok(match kind {
        ControlKind::Inspect => PreparedOperation::Inspect {
            request: InspectMachineRequest::new(operation_id.clone(), instance_id.clone()),
            instance_id,
        },
        ControlKind::Stop => PreparedOperation::Stop {
            request: StopMachineRequest::new(operation_id.clone(), instance_id.clone()),
            instance_id,
        },
        ControlKind::Destroy => PreparedOperation::Destroy {
            request: DestroyMachineRequest::new(operation_id.clone(), instance_id.clone()),
            instance_id,
        },
    })
}

fn identities(arguments: IdentityArgs) -> Result<(OperationId, InstanceId), RequestError> {
    Ok((
        operation_id(arguments.operation_id)?,
        arguments.instance_id.map_or_else(
            || InstanceId::new(generated_id()).map_err(|_| RequestError::Identity),
            |value| InstanceId::new(value).map_err(|_| RequestError::Identity),
        )?,
    ))
}

pub(crate) fn operation_id(value: Option<String>) -> Result<OperationId, RequestError> {
    value.map_or_else(
        || OperationId::new(generated_id()).map_err(|_| RequestError::Identity),
        |value| OperationId::new(value).map_err(|_| RequestError::Identity),
    )
}

fn generated_id() -> String {
    Uuid::new_v4().simple().to_string()
}

fn shape(arguments: ShapeArgs) -> Result<MachineShape, RequestError> {
    let capabilities =
        network::capabilities(arguments.network).map_err(|()| RequestError::Network)?;
    MachineShape::new(arguments.vcpus, arguments.memory_mib, arguments.storage_mib)
        .map(|shape| shape.with_capabilities(capabilities))
        .map_err(|_| RequestError::Shape)
}

fn command(mut values: Vec<String>) -> Result<DirectCommand, RequestError> {
    if values.is_empty() {
        return Err(RequestError::Command);
    }
    let arguments = values.split_off(1);
    DirectCommand::new(values.remove(0), arguments).map_err(|_| RequestError::Command)
}

fn limits(timeout_ms: u64, max_output_bytes: u64) -> Result<ExecutionLimits, RequestError> {
    ExecutionLimits::new(timeout_ms, max_output_bytes).map_err(|_| RequestError::Limits)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    Identity,
    Image,
    Name,
    Shape,
    Network,
    Command,
    Limits,
    Content,
    ContentTooLarge,
    Path,
}

impl RequestError {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Identity => "invalid_identity",
            Self::Image => "invalid_oci_image",
            Self::Name => "invalid_machine_name",
            Self::Shape => "invalid_machine_shape",
            Self::Network => "invalid_network_policy",
            Self::Command => "invalid_direct_command",
            Self::Limits => "invalid_execution_limits",
            Self::Content => "unreadable_content_file",
            Self::ContentTooLarge => "content_too_large",
            Self::Path => "invalid_guest_path",
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{PreparedOperation, RequestError, prepare_machine, prepare_run};
    use crate::cli::{Cli, RootCommand};

    #[test]
    fn prepares_facade_run_with_generated_canonical_identities() {
        let cli = Cli::try_parse_from([
            "soma",
            "run",
            "--network",
            "denied",
            "node:22",
            "--",
            "/usr/local/bin/node",
            "--version",
        ])
        .expect("run syntax");
        let RootCommand::Run(arguments) = cli.command else {
            panic!("run command");
        };
        let PreparedOperation::Run { instance_id, .. } =
            prepare_run(arguments).expect("facade request")
        else {
            panic!("run request");
        };
        assert_eq!(instance_id.as_str().len(), 32);
    }

    #[test]
    fn rejects_relative_guest_executable_before_runtime_work() {
        let cli = Cli::try_parse_from(["soma", "run", "node:22", "--", "node"])
            .expect("parser accepts bounded strings");
        let RootCommand::Run(arguments) = cli.command else {
            panic!("run command");
        };
        assert_eq!(prepare_run(arguments).err(), Some(RequestError::Command));
    }

    #[test]
    fn prepares_managed_control_with_explicit_operation_identity() {
        let cli = Cli::try_parse_from([
            "soma",
            "machine",
            "destroy",
            "--operation-id",
            "11111111111111111111111111111111",
            "--instance-id",
            "22222222222222222222222222222222",
        ])
        .expect("destroy syntax");
        let RootCommand::Machine(arguments) = cli.command else {
            panic!("machine command");
        };
        let PreparedOperation::Destroy { instance_id, .. } =
            prepare_machine(arguments).expect("destroy request")
        else {
            panic!("destroy request");
        };
        assert_eq!(instance_id.as_str(), "22222222222222222222222222222222");
    }
}
