//! One terminal answer as a bounded MCP tool result.

use rmcp::model::CallToolResult;
use serde::Serialize;

use super::{MCP_SCHEMA, ResultContractError};
use crate::{InstanceId, OperationId};

pub(crate) fn terminal_result(
    operation_id: &OperationId,
    expected_instance_id: &InstanceId,
    result: &crate::TerminalResult,
) -> Result<CallToolResult, ResultContractError> {
    if result.instance_id() != expected_instance_id {
        return Err(ResultContractError);
    }
    let refused = result.refused();
    let value = serde_json::to_value(TerminalEnvelope {
        schema: MCP_SCHEMA,
        operation: "terminal",
        operation_id: operation_id.as_str(),
        result: result.body(),
    })
    .map_err(|_| ResultContractError)?;
    Ok(if refused {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    })
}

#[derive(Serialize)]
struct TerminalEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: &'a str,
    result: crate::terminal::TerminalBody<'a>,
}
