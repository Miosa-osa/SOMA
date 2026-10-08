use serde_json::Value;

use crate::runner::{idle::MAX_IDLE_TIMEOUT_SECONDS, public_wire::PlatformError};

/// The exec timeout when a request names none, as the fast lane applies it.
const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;
const MAX_EXEC_TIMEOUT_SECONDS: u64 = 86_400;

/// Create parameters that ask for something other than the bare prepared sandbox.
///
/// This is the fast-lane door's own list (`Web.SomaFastLane.Handler`'s `@unsupported_params`):
/// on that path these requests fell through to the standard pipeline, and the runner has none.
/// `cwd` is the one addition to that list: contract C2 names a create carrying it among the
/// requests the runner does not serve, and only this list can refuse it with that answer.
const UNSUPPORTED_CREATE_PARAMS: [&str; 16] = [
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
    "cwd",
];

/// Every top-level create field the platform defines.
///
/// The door this runner replaces ignored any key it did not read, so a misspelled field quietly
/// created a sandbox that honored none of it. This list is the platform's own create vocabulary,
/// taken from the three places that define it: the fast-lane door
/// (`Web.SomaFastLane.Handler.build_attrs/3` and `shape?/2`), the create-parameter module
/// (`Web.Controllers.Sandboxes.CreateParams`), and the published SDK's create body
/// (`sdks/typescript/src/resources/sandboxes.ts`). A key outside it is answered with a 400 that
/// names it, so the next misspelled field is a refusal instead of a silent create.
const CREATE_FIELDS: [&str; 60] = [
    "__soma_api_started_at_us",
    "agent_runtime_profile_id",
    "allow_provision",
    "always_on",
    "auto_start",
    "compute_placement_request",
    "cpu_count",
    "cwd",
    "database",
    "depth",
    "disk_mb",
    "disk_size_mb",
    "entrypoint",
    "env",
    "external_project_id",
    "external_user_id",
    "external_workspace_id",
    "github_branch",
    "github_clone_path",
    "github_repo_url",
    "idle_timeout_sec",
    "image",
    "install",
    "install_command",
    "install_timeout_sec",
    "memory_mb",
    "metadata",
    "name",
    "network_profile",
    "persistent",
    "port",
    "project_id",
    "project_name",
    "project_slug",
    "readiness_probe",
    "region",
    "response_format",
    "revision",
    "runtime_profile",
    "services",
    "size",
    "skip_agent_runtime_profile",
    "slug",
    "snapshot",
    "snapshot_id",
    "source",
    "source_path",
    "source_snapshot_id",
    "start_command",
    "start_timeout_sec",
    "storage_profile",
    "tags",
    "template_id",
    "timeout",
    "timeout_sec",
    "wait",
    "workdir",
    "workspace_id",
    "workspace_name",
    "workspace_slug",
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
        refuse_unknown_fields(&params)?;
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
        let requested = params
            .get("timeout_sec")
            .filter(|value| !value.is_null())
            .or_else(|| params.get("timeout"));
        let timeout_seconds = match requested {
            None | Some(Value::Null) => default_timeout,
            Some(value) => value
                .as_u64()
                .filter(|seconds| *seconds <= MAX_IDLE_TIMEOUT_SECONDS)
                .ok_or_else(|| {
                    PlatformError::invalid_param(
                        "timeout",
                        "timeout must be an integer between 0 and 86400 seconds; 0 is no idle timeout",
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

/// Refuses a top-level create field the platform does not define, naming it in the answer.
fn refuse_unknown_fields(params: &serde_json::Map<String, Value>) -> Result<(), PlatformError> {
    for name in params.keys() {
        if !CREATE_FIELDS.contains(&name.as_str()) {
            return Err(PlatformError::unknown_field(name));
        }
    }
    Ok(())
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
