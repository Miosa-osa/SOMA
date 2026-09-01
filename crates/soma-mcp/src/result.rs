use base64::{Engine as _, engine::general_purpose::STANDARD};
use rmcp::model::CallToolResult;
use serde::Serialize;

use crate::{CommandResult, InspectResult, InstanceId, MachineResult, OperationId};

pub(crate) const MCP_SCHEMA: &str = "soma.mcp.v1";

mod file;

pub(crate) use file::file_result;

#[derive(Serialize)]
struct DoctorEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: Option<&'a str>,
    result: &'a crate::DoctorReport,
    receipt: Option<&'a serde_json::Value>,
}

#[derive(Serialize)]
struct FailureEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: Option<&'a str>,
    result: Option<CommandBody<'a>>,
    error: FailureBody<'a>,
    receipt: Option<&'a serde_json::Value>,
}

#[derive(Serialize)]
struct FailureBody<'a> {
    code: &'static str,
    message: &'static str,
    retryable: bool,
    instance_id: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub(crate) struct FailureDescriptor {
    code: &'static str,
    message: &'static str,
    retryable: bool,
}

impl FailureDescriptor {
    pub(crate) const fn new(code: &'static str, message: &'static str, retryable: bool) -> Self {
        Self {
            code,
            message,
            retryable,
        }
    }
}

#[derive(Serialize)]
struct CommandEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: &'a str,
    result: CommandBody<'a>,
    receipt: &'a serde_json::Value,
}

#[derive(Serialize)]
struct CommandBody<'a> {
    instance_id: &'a str,
    status: crate::CommandStatus,
    stdout: EncodedBytes,
    stderr: EncodedBytes,
}

#[derive(Serialize)]
struct MachineEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: &'a str,
    result: MachineBody<'a>,
    receipt: &'a serde_json::Value,
}

#[derive(Serialize)]
struct MachineBody<'a> {
    instance_id: &'a str,
    state: crate::MachineState,
}

#[derive(Serialize)]
struct InspectEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: Option<&'a str>,
    result: InspectBody<'a>,
    receipt: Option<&'a serde_json::Value>,
}

#[derive(Serialize)]
struct InspectBody<'a> {
    instance_id: &'a str,
    state: crate::MachineState,
    backend: crate::BackendTarget,
}

#[derive(Serialize)]
struct EncodedBytes {
    encoding: &'static str,
    byte_length: usize,
    data: String,
}

impl EncodedBytes {
    fn new(bytes: &[u8]) -> Self {
        Self {
            encoding: "base64",
            byte_length: bytes.len(),
            data: STANDARD.encode(bytes),
        }
    }
}

pub(crate) fn doctor_result(
    report: &crate::DoctorReport,
) -> Result<CallToolResult, ResultContractError> {
    serde_json::to_value(DoctorEnvelope {
        schema: MCP_SCHEMA,
        operation: "doctor",
        operation_id: None,
        result: report,
        receipt: None,
    })
    .map(CallToolResult::structured)
    .map_err(|_| ResultContractError)
}

pub(crate) fn failure_result(
    operation: &'static str,
    operation_id: Option<&OperationId>,
    instance_id: Option<&InstanceId>,
    descriptor: FailureDescriptor,
    failure: &crate::RuntimeFailure,
    max_output_bytes: Option<u64>,
) -> Result<CallToolResult, ResultContractError> {
    let result = failure
        .command()
        .map(|command| {
            let instance_id = instance_id.ok_or(ResultContractError)?;
            let allowance = max_output_bytes.ok_or(ResultContractError)?;
            validate_output_bound(command.stdout(), command.stderr(), allowance)?;
            Ok(CommandBody {
                instance_id: instance_id.as_str(),
                status: command.status(),
                stdout: EncodedBytes::new(command.stdout()),
                stderr: EncodedBytes::new(command.stderr()),
            })
        })
        .transpose()?;
    serde_json::to_value(FailureEnvelope {
        schema: MCP_SCHEMA,
        operation,
        operation_id: operation_id.map(OperationId::as_str),
        result,
        error: FailureBody {
            code: descriptor.code,
            message: descriptor.message,
            retryable: descriptor.retryable,
            instance_id: instance_id.map(InstanceId::as_str),
        },
        receipt: failure.receipt().map(crate::ExecutionReceipt::as_value),
    })
    .map(CallToolResult::structured_error)
    .map_err(|_| ResultContractError)
}

pub(crate) fn command_result(
    operation: &'static str,
    operation_id: &OperationId,
    expected_instance_id: &InstanceId,
    result: &CommandResult,
    max_output_bytes: u64,
) -> Result<CallToolResult, ResultContractError> {
    if result.instance_id() != expected_instance_id {
        return Err(ResultContractError);
    }
    validate_output_bound(result.stdout(), result.stderr(), max_output_bytes)?;
    let stdout = EncodedBytes::new(result.stdout());
    let stderr = EncodedBytes::new(result.stderr());
    let envelope = CommandEnvelope {
        schema: MCP_SCHEMA,
        operation,
        operation_id: operation_id.as_str(),
        result: CommandBody {
            instance_id: result.instance_id().as_str(),
            status: result.status(),
            stdout,
            stderr,
        },
        receipt: result.receipt().as_value(),
    };
    serde_json::to_value(envelope)
        .map(CallToolResult::structured)
        .map_err(|_| ResultContractError)
}

fn validate_output_bound(
    stdout: &[u8],
    stderr: &[u8],
    max_output_bytes: u64,
) -> Result<(), ResultContractError> {
    let combined = stdout
        .len()
        .checked_add(stderr.len())
        .ok_or(ResultContractError)?;
    let combined = u64::try_from(combined).map_err(|_| ResultContractError)?;
    if combined > max_output_bytes || combined > crate::ExecutionLimits::MAX_OUTPUT_BYTES {
        return Err(ResultContractError);
    }
    Ok(())
}

pub(crate) fn machine_result(
    operation: &'static str,
    operation_id: &OperationId,
    expected_instance_id: &InstanceId,
    result: &MachineResult,
) -> Result<CallToolResult, ResultContractError> {
    if result.instance_id() != expected_instance_id {
        return Err(ResultContractError);
    }
    serde_json::to_value(MachineEnvelope {
        schema: MCP_SCHEMA,
        operation,
        operation_id: operation_id.as_str(),
        result: MachineBody {
            instance_id: result.instance_id().as_str(),
            state: result.state(),
        },
        receipt: result.receipt().as_value(),
    })
    .map(CallToolResult::structured)
    .map_err(|_| ResultContractError)
}

pub(crate) fn inspect_result(
    operation_id: &OperationId,
    expected_instance_id: &InstanceId,
    result: &InspectResult,
) -> Result<CallToolResult, ResultContractError> {
    if result.instance_id() != expected_instance_id {
        return Err(ResultContractError);
    }
    serde_json::to_value(InspectEnvelope {
        schema: MCP_SCHEMA,
        operation: "inspect",
        operation_id: Some(operation_id.as_str()),
        result: InspectBody {
            instance_id: result.instance_id().as_str(),
            state: result.state(),
            backend: result.backend(),
        },
        receipt: result.receipt().map(crate::ExecutionReceipt::as_value),
    })
    .map(CallToolResult::structured)
    .map_err(|_| ResultContractError)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResultContractError;
