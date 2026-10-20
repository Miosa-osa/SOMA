use std::fmt;

use serde_json::Value;
use soma::{Capabilities, DirectCommand, MachineShape};

use crate::runner::{
    config::{LargeShape, LaunchConfig},
    idle::MAX_IDLE_TIMEOUT_SECONDS,
    public_wire::PlatformError,
};

mod exec;
mod fields;
mod run;

use fields::{CREATE_FIELDS, UNSUPPORTED_CREATE_PARAMS};

pub(super) use exec::ExecParams;
pub(super) use run::RunCommand;

/// The create body fields the runner acts on.
#[derive(PartialEq, Eq)]
pub(super) struct CreateParams {
    pub(super) project_id: Option<String>,
    pub(super) timeout_seconds: u64,
    /// The machine this create launches: the shape its `size` selected.
    ///
    /// The one shape a create may always have is the runner's own, so a request that names no
    /// size, or names `xs`, carries exactly the shape the host was sized and prewarmed for.
    /// `large` carries the runner's large shape, and is only reachable on a runner that
    /// configured one.
    pub(super) shape: MachineShape,
    /// The egress the create named in `network.egress`, if it named one.
    ///
    /// Already applied to [`Self::shape`]: this sandbox's network is what the request said, not
    /// what the host's `launch.public_egress` defaults a shape to. Kept so a launch that cannot
    /// serve it is answered as an egress refusal rather than as a generic outage.
    pub(super) egress: Option<EgressRequest>,
    /// Whether the sandbox is created with a denied network whose egress can be opened later.
    ///
    /// Only meaningful with `egress: denied`; the sandbox then holds a broker bundle from boot.
    pub(super) windowed: bool,
    /// Whether the create waits for the sandbox to be up before it answers.
    ///
    /// Absent is `true`, the answer every client had before this field existed. `false` is the
    /// early `201`: the runner answers as soon as the launch has taken a facade lease and keeps
    /// bringing the sandbox up behind the answer, and every later call on it waits.
    pub(super) wait_ready: bool,
    /// The command a create-and-run carries, already through every check the exec door makes.
    ///
    /// A create that carries one always waits for its sandbox first, so this and
    /// [`Self::wait_ready`] are never both set away from their defaults.
    pub(super) run: Option<RunCommand>,
}

/// The command is not printed: a create's command is the caller's, and a debug print of it in a
/// test failure or a log line is a copy of it in a place it was not sent to.
impl fmt::Debug for CreateParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateParams")
            .field("project_id", &self.project_id)
            .field("timeout_seconds", &self.timeout_seconds)
            .field("shape", &self.shape)
            .field("egress", &self.egress)
            .field("windowed", &self.windowed)
            .field("wait_ready", &self.wait_ready)
            .field("run", &self.run.is_some())
            .finish()
    }
}

/// What a create's `network.egress` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EgressRequest {
    /// No TCP, no UDP, no DNS.
    Denied,
    /// Public destinations only, with the system resolvers.
    Internet,
    /// The operator-admitted unfiltered network.
    Unrestricted,
}

impl EgressRequest {
    /// The word the create body used and its answer echoes.
    pub(super) const fn word(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Internet => "internet",
            Self::Unrestricted => "unrestricted",
        }
    }

    /// Whether serving it needs the host's network broker. A windowed `denied` does too, which
    /// the caller adds.
    pub(super) const fn needs_broker(self) -> bool {
        !matches!(self, Self::Denied)
    }

    fn capabilities(self) -> Capabilities {
        match self {
            Self::Denied => Capabilities::isolated(),
            Self::Internet => Capabilities::isolated().with_network_access(),
            Self::Unrestricted => Capabilities::isolated().with_unrestricted_network(),
        }
    }
}

/// Applies a create's `network` object to the shape it selected.
///
/// The object has one key, `egress`, and an unknown key is refused so a misspelling is never
/// served as the default network. A create that names no network keeps the host's own default,
/// which is how every create behaved before this field existed.
fn apply_network(
    shape: MachineShape,
    network: Option<&Value>,
    launch: &LaunchConfig,
) -> Result<(MachineShape, Option<EgressRequest>, bool), PlatformError> {
    let invalid = |detail: &str| PlatformError::invalid_param("network", detail);
    let object = match network {
        None | Some(Value::Null) => return Ok((shape, None, false)),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(invalid("network must be an object")),
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "egress" | "windowed"))
    {
        return Err(invalid(&format!("network has no `{unknown}` field")));
    }
    let egress = match object.get("egress").map(Value::as_str) {
        Some(Some("denied")) => EgressRequest::Denied,
        Some(Some("internet")) => EgressRequest::Internet,
        Some(Some("unrestricted")) => EgressRequest::Unrestricted,
        _ => {
            return Err(invalid(
                "network.egress must be \"denied\", \"internet\" or \"unrestricted\"",
            ));
        }
    };
    let windowed = match object.get("windowed") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(windowed)) => *windowed,
        Some(_) => return Err(invalid("network.windowed must be a boolean")),
    };
    // A window opens the internet on a sandbox that starts with none, so it only makes sense
    // there; asking for one next to an already open network is a mistake worth refusing.
    if windowed && egress != EgressRequest::Denied {
        return Err(invalid("network.windowed needs network.egress \"denied\""));
    }
    if egress == EgressRequest::Unrestricted && !launch.allow_unrestricted_egress {
        return Err(PlatformError::new(
            403,
            "egress_not_permitted",
            "this runner does not serve unrestricted egress",
            false,
        ));
    }
    Ok((
        shape.with_capabilities(egress.capabilities()),
        Some(egress),
        windowed,
    ))
}

impl CreateParams {
    /// Accepts the bare prepared-sandbox create the fast lane served, and refuses the rest.
    pub(super) fn parse(
        body: &[u8],
        default_timeout: u64,
        launch: &LaunchConfig,
        shell_free_exec: bool,
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
        // The size vocabulary is closed and small: the runner's own shape, and the large one
        // when this runner configured it. Anything else, including a size this platform names
        // but this host cannot build, is refused by name rather than served as another size.
        let shape = match params.get("size").map(Value::as_str) {
            None | Some(Some("xs")) => launch.shape.clone(),
            Some(Some("large")) => launch
                .large
                .as_ref()
                .map(LargeShape::machine_shape)
                .ok_or_else(|| PlatformError::unsupported("a size other than xs"))?,
            Some(_) => return Err(PlatformError::unsupported("a size other than xs or large")),
        };
        let (shape, egress, windowed) = apply_network(shape, params.get("network"), launch)?;
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
        let run = RunCommand::parse(&params, shell_free_exec)?;
        let wait_ready = wait_ready(&params)?;
        if run.is_some() && !wait_ready {
            // The command cannot run before the sandbox is up, so a create that carries one has
            // nothing to answer early about: it waits, and says so rather than silently ignoring
            // the field it was given.
            return Err(PlatformError::invalid_param(
                "wait_ready",
                "a create that runs a command waits for its sandbox; send `wait_ready` as true or leave it out",
            ));
        }
        Ok(Self {
            project_id,
            timeout_seconds,
            shape,
            egress,
            windowed,
            wait_ready,
            run,
        })
    }
}

/// Whether a create waits for its sandbox to be up, read the way the platform reads its
/// booleans: absent or null is the synchronous answer that predates this field, and `true`,
/// `false`, `"true"`, `"1"`, `"false"` and `"0"` are the values a form or a query string sends.
fn wait_ready(params: &serde_json::Map<String, Value>) -> Result<bool, PlatformError> {
    match params.get("wait_ready") {
        None | Some(Value::Null) => Ok(true),
        Some(value) if truthy(value) => Ok(true),
        Some(value) if falsy(value) => Ok(false),
        Some(_) => Err(PlatformError::invalid_param(
            "wait_ready",
            "wait_ready must be a boolean",
        )),
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

/// `Handler.falsy?/1`: the values the platform reads as a `false` a human typed into a form.
fn falsy(value: &Value) -> bool {
    matches!(value, Value::Bool(false)) || matches!(value.as_str(), Some("false" | "0"))
}
