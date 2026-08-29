use super::{DestroyInput, ExecInput, InputError, InspectInput, LaunchInput, RunInput, StopInput};
use crate::{
    DestroyRequest, DirectCommand, DisplayName, ExecRequest, ExecutionLimits, InspectRequest,
    InstanceId, LaunchRequest, MachineShape, OciImage, OperationId, RunRequest, StopRequest,
};

impl TryFrom<RunInput> for RunRequest {
    type Error = InputError;

    fn try_from(input: RunInput) -> Result<Self, Self::Error> {
        let capabilities = input.network.capabilities()?;
        Ok(Self::new(
            optional_operation_id(input.operation_id)?,
            optional_instance_id(input.instance_id)?,
            crate::request::MachineDefinition::new(
                OciImage::parse(input.image).map_err(|_| InputError::Image)?,
                optional_display_name(input.display_name)?,
                MachineShape::new(input.vcpu_count, input.memory_mib, input.storage_mib)
                    .map(|shape| shape.with_capabilities(capabilities))
                    .map_err(|_| InputError::Shape)?,
                input.backend.into(),
            ),
            DirectCommand::new(input.executable, input.arguments)
                .map_err(|_| InputError::Command)?,
            ExecutionLimits::new(input.timeout_ms, input.max_output_bytes)
                .map_err(|_| InputError::Limits)?,
        ))
    }
}

impl TryFrom<LaunchInput> for LaunchRequest {
    type Error = InputError;

    fn try_from(input: LaunchInput) -> Result<Self, Self::Error> {
        let capabilities = input.network.capabilities()?;
        Ok(Self::new(
            optional_operation_id(input.operation_id)?,
            optional_instance_id(input.instance_id)?,
            crate::request::MachineDefinition::new(
                OciImage::parse(input.image).map_err(|_| InputError::Image)?,
                optional_display_name(input.display_name)?,
                MachineShape::new(input.vcpu_count, input.memory_mib, input.storage_mib)
                    .map(|shape| shape.with_capabilities(capabilities))
                    .map_err(|_| InputError::Shape)?,
                input.backend.into(),
            ),
        ))
    }
}

impl TryFrom<ExecInput> for ExecRequest {
    type Error = InputError;

    fn try_from(input: ExecInput) -> Result<Self, Self::Error> {
        Ok(Self::new(
            optional_operation_id(input.operation_id)?,
            required_instance_id(input.instance_id)?,
            DirectCommand::new(input.executable, input.arguments)
                .map_err(|_| InputError::Command)?,
            ExecutionLimits::new(input.timeout_ms, input.max_output_bytes)
                .map_err(|_| InputError::Limits)?,
            input.backend.into(),
        ))
    }
}

macro_rules! lifecycle_request {
    ($input:ty, $request:ty) => {
        impl TryFrom<$input> for $request {
            type Error = InputError;

            fn try_from(input: $input) -> Result<Self, Self::Error> {
                Ok(Self::new(
                    optional_operation_id(input.operation_id)?,
                    required_instance_id(input.instance_id)?,
                    input.backend.into(),
                ))
            }
        }
    };
}

lifecycle_request!(InspectInput, InspectRequest);
lifecycle_request!(StopInput, StopRequest);
lifecycle_request!(DestroyInput, DestroyRequest);

fn optional_operation_id(value: Option<String>) -> Result<OperationId, InputError> {
    value.map_or_else(
        || Ok(crate::identity::generate_operation_id()),
        |value| OperationId::new(value).map_err(|_| InputError::OperationId),
    )
}

fn optional_instance_id(value: Option<String>) -> Result<InstanceId, InputError> {
    value.map_or_else(
        || Ok(crate::identity::generate_instance_id()),
        |value| InstanceId::new(value).map_err(|_| InputError::InstanceId),
    )
}

fn required_instance_id(value: String) -> Result<InstanceId, InputError> {
    InstanceId::new(value).map_err(|_| InputError::InstanceId)
}

fn optional_display_name(value: Option<String>) -> Result<Option<DisplayName>, InputError> {
    value
        .map(|value| DisplayName::parse(value).map_err(|_| InputError::DisplayName))
        .transpose()
}
