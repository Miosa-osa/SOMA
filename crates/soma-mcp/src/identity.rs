pub use soma::{InstanceId, OperationId};

#[must_use]
pub(crate) fn generate_operation_id() -> OperationId {
    OperationId::new(uuid::Uuid::new_v4().simple().to_string())
        .expect("UUIDv4 simple form is a canonical operation ID")
}

#[must_use]
pub(crate) fn generate_instance_id() -> InstanceId {
    InstanceId::new(uuid::Uuid::new_v4().simple().to_string())
        .expect("UUIDv4 simple form is a canonical instance ID")
}
