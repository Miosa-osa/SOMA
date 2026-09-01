//! One filesystem answer as a bounded tool result.
//!
//! It is apart from the lifecycle results because it is the only one whose envelope can be
//! either a success or an error for the same reason: the guest was reached either way, and what
//! differs is whether it declined.

use rmcp::model::CallToolResult;
use serde::Serialize;

use super::{MCP_SCHEMA, ResultContractError};
use crate::{InstanceId, OperationId};

/// One filesystem answer as a bounded tool result.
///
/// A refusal the guest reported is returned as a tool error rather than as a success, because an
/// agent that read "the operation happened" from a declined write would go on to depend on a
/// file that is not there. The typed cause travels in the result either way.
pub(crate) fn file_result(
    operation_id: &OperationId,
    expected_instance_id: &InstanceId,
    result: &crate::FileResult,
) -> Result<CallToolResult, ResultContractError> {
    if result.instance_id() != expected_instance_id {
        return Err(ResultContractError);
    }
    let refused = result.refused();
    let value = serde_json::to_value(FileEnvelope {
        schema: MCP_SCHEMA,
        operation: "file",
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
struct FileEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: &'a str,
    result: crate::file::FileBody<'a>,
}
