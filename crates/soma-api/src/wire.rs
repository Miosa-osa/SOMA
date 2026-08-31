use serde::Deserialize;
use soma::{
    DestroyMachineRequest, DirectCommand, ExecuteMachineRequest, ExecutionLimits,
    InspectMachineRequest, InstanceId, LaunchMachineRequest, MachineName, MachineShape, OciImage,
    OperationId, StopMachineRequest,
};

use crate::envelope::ApiError;

/// The body of a create-sandbox request.
///
/// `shape`, `name`, `instance_id`, and `operation_id` deserialize straight into the portable
/// facade's own types, so their validation is the facade's validation and cannot drift. `image`
/// arrives as a string only because `OciImage` publishes a parser rather than a deserializer;
/// the parser is still what accepts or rejects it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSandboxBody {
    pub image: String,
    #[serde(default)]
    pub shape: Option<MachineShape>,
    #[serde(default)]
    pub name: Option<MachineName>,
    #[serde(default)]
    pub instance_id: Option<InstanceId>,
    #[serde(default)]
    pub operation_id: Option<OperationId>,
}

/// The body of a run-command request.
///
/// The command is split into executable and arguments exactly as `DirectCommand` requires. There
/// is no shell string field, because the facade never interposes a shell and accepting one would
/// promise a behavior the engine does not have.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCommandBody {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub limits: Option<ExecutionLimits>,
    #[serde(default)]
    pub operation_id: Option<OperationId>,
}

/// The body accepted by the lifecycle control routes that need no other input.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlBody {
    #[serde(default)]
    pub operation_id: Option<OperationId>,
}

impl CreateSandboxBody {
    /// Converts the parsed body into the facade's launch request.
    ///
    /// # Errors
    ///
    /// Returns a 400 refusal when the image reference or generated identity is not acceptable to
    /// the facade.
    pub fn into_facade(self) -> Result<(InstanceId, LaunchMachineRequest), ApiError> {
        let image = OciImage::parse(self.image)
            .map_err(|_| ApiError::invalid("the image reference is not a valid OCI reference"))?;
        let instance_id = match self.instance_id {
            Some(instance_id) => instance_id,
            None => generated_instance_id()?,
        };
        let operation_id = operation_id(self.operation_id)?;
        let shape = self.shape.unwrap_or_else(default_shape);
        let mut request =
            LaunchMachineRequest::new(operation_id, instance_id.clone(), image, shape);
        if let Some(name) = self.name {
            request = request.with_name(name);
        }
        Ok((instance_id, request))
    }
}

impl RunCommandBody {
    /// Converts the parsed body into the facade's execute request.
    ///
    /// # Errors
    ///
    /// Returns a 400 refusal when the command violates the facade's direct-command bounds.
    pub fn into_facade(self, instance_id: InstanceId) -> Result<ExecuteMachineRequest, ApiError> {
        let command = DirectCommand::new(self.executable, self.arguments).map_err(|_| {
            ApiError::invalid(
                "the command must name an absolute executable within the facade's bounds",
            )
        })?;
        let limits = self.limits.unwrap_or_else(default_limits);
        Ok(ExecuteMachineRequest::new(
            operation_id(self.operation_id)?,
            instance_id,
            command,
            limits,
        ))
    }
}

impl ControlBody {
    /// Builds an inspect request for one instance.
    ///
    /// # Errors
    ///
    /// Returns a 500 refusal only if a generated operation identity is rejected by the facade.
    pub fn into_inspect(self, instance_id: InstanceId) -> Result<InspectMachineRequest, ApiError> {
        Ok(InspectMachineRequest::new(
            operation_id(self.operation_id)?,
            instance_id,
        ))
    }

    /// Builds a stop request for one instance.
    ///
    /// # Errors
    ///
    /// Returns a 500 refusal only if a generated operation identity is rejected by the facade.
    pub fn into_stop(self, instance_id: InstanceId) -> Result<StopMachineRequest, ApiError> {
        Ok(StopMachineRequest::new(
            operation_id(self.operation_id)?,
            instance_id,
        ))
    }

    /// Builds a destroy request for one instance.
    ///
    /// # Errors
    ///
    /// Returns a 500 refusal only if a generated operation identity is rejected by the facade.
    pub fn into_destroy(self, instance_id: InstanceId) -> Result<DestroyMachineRequest, ApiError> {
        Ok(DestroyMachineRequest::new(
            operation_id(self.operation_id)?,
            instance_id,
        ))
    }
}

/// Parses an instance id taken from a request path.
///
/// # Errors
///
/// Returns a 404 refusal rather than a 400, because a path segment that cannot be an instance id
/// can never name an existing sandbox, and reporting it as absent leaks nothing about which ids
/// do exist.
pub fn path_instance_id(segment: &str) -> Result<InstanceId, ApiError> {
    InstanceId::new(segment).map_err(|_| {
        ApiError::new(
            404,
            "machine_not_found",
            "sandbox instance was not found",
            false,
        )
    })
}

fn operation_id(supplied: Option<OperationId>) -> Result<OperationId, ApiError> {
    match supplied {
        Some(operation_id) => Ok(operation_id),
        None => OperationId::new(uuid::Uuid::new_v4().simple().to_string()).map_err(|_| {
            ApiError::internal("a generated operation identity was rejected by the facade")
        }),
    }
}

fn generated_instance_id() -> Result<InstanceId, ApiError> {
    InstanceId::new(uuid::Uuid::new_v4().simple().to_string())
        .map_err(|_| ApiError::internal("a generated instance identity was rejected by the facade"))
}

/// The shape used when a caller does not state one, matching the CLI's published defaults.
fn default_shape() -> MachineShape {
    MachineShape::new(
        MachineShape::DEFAULT_VCPU_COUNT,
        MachineShape::DEFAULT_MEMORY_MIB,
        MachineShape::DEFAULT_STORAGE_MIB,
    )
    .expect("the facade's own default shape is valid")
}

/// The limits used when a caller does not state them, matching the CLI's published defaults.
fn default_limits() -> ExecutionLimits {
    ExecutionLimits::new(
        ExecutionLimits::DEFAULT_TIMEOUT_MS,
        ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES,
    )
    .expect("the facade's own default limits are valid")
}
