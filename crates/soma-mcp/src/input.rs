use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BackendTarget, DirectCommand, DisplayName, ExecutionLimits, MachineShape};
mod conversion;
mod network;

use network::NetworkInput;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackendInput {
    #[default]
    Auto,
    Local,
    Kvm,
    Macos,
}

impl From<BackendInput> for BackendTarget {
    fn from(value: BackendInput) -> Self {
        match value {
            BackendInput::Auto => Self::Auto,
            BackendInput::Local => Self::Local,
            BackendInput::Kvm => Self::Kvm,
            BackendInput::Macos => Self::Macos,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DoctorInput {
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: Option<String>,
    #[schemars(length(min = 1, max = 1024))]
    pub image: String,
    #[schemars(length(min = DisplayName::MIN_BYTES, max = DisplayName::MAX_BYTES))]
    pub display_name: Option<String>,
    #[schemars(length(min = 1, max = DirectCommand::MAX_EXECUTABLE_BYTES))]
    pub executable: String,
    #[serde(default)]
    #[schemars(length(max = DirectCommand::MAX_ARGUMENTS), inner(length(max = DirectCommand::MAX_ARGUMENT_BYTES)))]
    pub arguments: Vec<String>,
    #[serde(default = "default_vcpu_count")]
    #[schemars(range(min = MachineShape::MIN_VCPU_COUNT))]
    pub vcpu_count: u16,
    #[serde(default = "default_memory_mib")]
    #[schemars(range(min = MachineShape::MIN_MEMORY_MIB))]
    pub memory_mib: u64,
    #[serde(default = "default_storage_mib")]
    #[schemars(range(min = MachineShape::MIN_STORAGE_MIB))]
    pub storage_mib: u64,
    #[serde(default)]
    #[schemars(default)]
    pub network: NetworkInput,
    #[serde(default = "default_timeout_ms")]
    #[schemars(range(min = ExecutionLimits::MIN_TIMEOUT_MS, max = ExecutionLimits::MAX_TIMEOUT_MS))]
    pub timeout_ms: u64,
    #[serde(default = "default_output_bytes")]
    #[schemars(range(min = ExecutionLimits::MIN_OUTPUT_BYTES, max = ExecutionLimits::MAX_OUTPUT_BYTES))]
    pub max_output_bytes: u64,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: Option<String>,
    #[schemars(length(min = 1, max = 1024))]
    pub image: String,
    #[schemars(length(min = DisplayName::MIN_BYTES, max = DisplayName::MAX_BYTES))]
    pub display_name: Option<String>,
    #[serde(default = "default_vcpu_count")]
    #[schemars(range(min = MachineShape::MIN_VCPU_COUNT))]
    pub vcpu_count: u16,
    #[serde(default = "default_memory_mib")]
    #[schemars(range(min = MachineShape::MIN_MEMORY_MIB))]
    pub memory_mib: u64,
    #[serde(default = "default_storage_mib")]
    #[schemars(range(min = MachineShape::MIN_STORAGE_MIB))]
    pub storage_mib: u64,
    #[serde(default)]
    #[schemars(default)]
    pub network: NetworkInput,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: String,
    #[schemars(length(min = 1, max = DirectCommand::MAX_EXECUTABLE_BYTES))]
    pub executable: String,
    #[serde(default)]
    #[schemars(length(max = DirectCommand::MAX_ARGUMENTS), inner(length(max = DirectCommand::MAX_ARGUMENT_BYTES)))]
    pub arguments: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    #[schemars(range(min = ExecutionLimits::MIN_TIMEOUT_MS, max = ExecutionLimits::MAX_TIMEOUT_MS))]
    pub timeout_ms: u64,
    #[serde(default = "default_output_bytes")]
    #[schemars(range(min = ExecutionLimits::MIN_OUTPUT_BYTES, max = ExecutionLimits::MAX_OUTPUT_BYTES))]
    pub max_output_bytes: u64,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InspectInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: String,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StopInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: String,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DestroyInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: String,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

const fn default_vcpu_count() -> u16 {
    MachineShape::DEFAULT_VCPU_COUNT
}

const fn default_memory_mib() -> u64 {
    MachineShape::DEFAULT_MEMORY_MIB
}

const fn default_storage_mib() -> u64 {
    MachineShape::DEFAULT_STORAGE_MIB
}

const fn default_timeout_ms() -> u64 {
    ExecutionLimits::DEFAULT_TIMEOUT_MS
}

const fn default_output_bytes() -> u64 {
    ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputError {
    OperationId,
    InstanceId,
    DisplayName,
    Image,
    Command,
    Shape,
    Network,
    Limits,
}

impl InputError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::OperationId => "operation_id must be 32 nonzero lowercase hexadecimal characters",
            Self::InstanceId => "instance_id must be 32 nonzero lowercase hexadecimal characters",
            Self::DisplayName => {
                "display_name must be 1 to 63 lowercase alphanumeric or hyphen bytes with alphanumeric ends"
            }
            Self::Image => "image must be a bounded OCI reference without a URL scheme",
            Self::Command => "command must satisfy the bounded direct argv contract",
            Self::Shape => "machine shape is outside the supported request bounds",
            Self::Network => "network policy is invalid or contradictory",
            Self::Limits => "execution limits are outside the supported request bounds",
        }
    }
}
