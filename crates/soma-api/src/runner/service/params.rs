use serde_json::Value;

use crate::runner::public_wire::PlatformError;

/// The exec timeout when a request names none, as the fast lane applies it.
const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;
const MAX_EXEC_TIMEOUT_SECONDS: u64 = 86_400;

/// Create parameters that ask for something other than the bare prepared sandbox.
///
/// This is the fast-lane door's own list (`Web.SomaFastLane.Handler`'s `@unsupported_params`):
/// on that path these requests fell through to the standard pipeline, and the runner has none.
const UNSUPPORTED_CREATE_PARAMS: [&str; 15] = [
    "snapshot_id",
    "source_snapshot_id",
    "snapshot",
    "source",
    "github_repo_url",
    "template_id",
    "image",
    "env",
    "entrypoint",
    "services",
    "database",
    "name",
    "readiness_probe",
    "always_on",
    "slug",
];

/// The create body fields the runner acts on.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CreateParams {
    pub(super) project_id: Option<String>,
    pub(super) timeout_seconds: u64,
}

impl CreateParams {
    /// Accepts the bare prepared-sandbox create the fast lane served, and refuses the rest.
    pub(super) fn parse(
        body: &[u8],
        default_timeout: u64,
        shape: &soma::MachineShape,
    ) -> Result<Self, PlatformError> {
        let params = object(body)?;
        for name in UNSUPPORTED_CREATE_PARAMS {
            if !empty(params.get(name)) {
                return Err(PlatformError::unsupported(&format!(
                    "the `{name}` parameter"
                )));
            }
        }
        if !matches!(
            params.get("size").map(Value::as_str),
            None | Some(Some("xs"))
        ) {
            return Err(PlatformError::unsupported("a size other than xs"));
        }
        if params.get("persistent").is_some_and(truthy) {
            return Err(PlatformError::unsupported("persistent sandboxes"));
        }
        if params.get("auto_start").is_some_and(truthy) {
            return Err(PlatformError::unsupported("auto_start"));
        }
        if !matches!(
            params.get("runtime_profile").map(Value::as_str),
            None | Some(Some("soma"))
        ) {
            return Err(PlatformError::unsupported(
                "a runtime profile other than soma",
            ));
        }
        for (field, configured) in [
            ("cpu_count", u64::from(shape.vcpu_count())),
            ("memory_mb", shape.memory_mib()),
        ] {
            if let Some(requested) = params.get(field).filter(|value| !value.is_null())
                && requested.as_u64() != Some(configured)
            {
                return Err(PlatformError::unsupported(&format!(
                    "a `{field}` other than this runner's {configured}"
                )));
            }
        }
        let project_id = match params.get("project_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(project)) if project.is_empty() => None,
            Some(Value::String(project)) => Some(project.clone()),
            Some(_) => {
                return Err(PlatformError::invalid_param(
                    "project_id",
                    "project_id must be a string",
                ));
            }
        };
        let timeout_seconds = match params.get("timeout_sec") {
            None | Some(Value::Null) => default_timeout,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_EXEC_TIMEOUT_SECONDS).contains(seconds))
                .ok_or_else(|| {
                    PlatformError::invalid_param(
                        "timeout_sec",
                        "timeout_sec must be an integer between 1 and 86400",
                    )
                })?,
        };
        Ok(Self {
            project_id,
            timeout_seconds,
        })
    }
}

/// The exec body fields the fast lane accepted.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ExecParams {
    pub(super) command: String,
    pub(super) timeout_ms: u64,
}

impl ExecParams {
    pub(super) fn parse(body: &[u8]) -> Result<Self, PlatformError> {
        let params = object(body)?;
        let command = match params.get("command") {
            Some(Value::String(command)) if !command.is_empty() => command.clone(),
            _ => return Err(PlatformError::missing_command()),
        };
        if !params.get("cwd").is_none_or(Value::is_null) {
            return Err(PlatformError::unsupported("a working directory (`cwd`)"));
        }
        let timeout_ms = match params.get("timeout") {
            None | Some(Value::Null) => DEFAULT_EXEC_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_EXEC_TIMEOUT_SECONDS).contains(seconds))
                .map(|seconds| seconds * 1_000)
                .ok_or_else(PlatformError::invalid_timeout)?,
        };
        Ok(Self {
            command,
            timeout_ms,
        })
    }
}

/// A JSON object body; an empty body is an empty object, as `decode_json_object("")` made it.
fn object(body: &[u8]) -> Result<serde_json::Map<String, Value>, PlatformError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(serde_json::Map::new());
    }
    match serde_json::from_slice(body) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(PlatformError::new(
            400,
            "INVALID_JSON",
            "the request body must be a JSON object",
            false,
        )),
    }
}

/// `Handler.empty?/1`: absent, null, `""`, `[]`, `{}`, or `false`.
fn empty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(fields)) => fields.is_empty(),
        Some(_) => false,
    }
}

/// `Handler.truthy?/1`: `true`, `"true"`, or `"1"`.
fn truthy(value: &Value) -> bool {
    matches!(value, Value::Bool(true)) || matches!(value.as_str(), Some("true" | "1"))
}
