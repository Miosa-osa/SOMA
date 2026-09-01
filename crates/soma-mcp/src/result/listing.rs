//! One deterministic sandbox listing as an MCP tool result.

use rmcp::model::CallToolResult;
use serde::Serialize;

use super::{MCP_SCHEMA, ResultContractError};

pub(crate) fn list_result(
    result: &crate::ListResult,
) -> Result<CallToolResult, ResultContractError> {
    serde_json::to_value(ListEnvelope {
        schema: MCP_SCHEMA,
        operation: "list",
        operation_id: None,
        result: result.body(),
    })
    .map(CallToolResult::structured)
    .map_err(|_| ResultContractError)
}

#[derive(Serialize)]
struct ListEnvelope<'a> {
    schema: &'static str,
    operation: &'static str,
    operation_id: Option<&'a str>,
    result: crate::listing::ListBody<'a>,
}
