#![doc = "Template parsing, module composition, and canonical Template Lock construction for SOMA."]
#![forbid(unsafe_code)]

mod compose;
mod error;
mod identity;
mod lock;
mod module;
mod rejection;
mod resolve;
mod revision;
mod schema;
mod validate;
mod wire;

pub use error::{BoundError, ExternalDependency, LockError, ParseError, TemplateError};
pub use identity::{LockId, LockIdError};
pub use lock::{
    LOCK_MAGIC, LOCK_SCHEMA_VERSION, LockedCommand, LockedEnvironment, LockedModule, LockedSecret,
    MAX_LOCK_ENVIRONMENT, MAX_LOCK_MODULES, POLICY_VERSION, TemplateLock,
};
pub use module::{
    Destination, EnvironmentName, GuestPath, HealthProbe, MAX_DESTINATION_HOST_BYTES,
    MAX_ENVIRONMENT_NAME_BYTES, MAX_FIELD_NAME_BYTES, MAX_MODULE_LIST, MAX_MODULE_NAME_BYTES,
    MAX_PATH_BYTES, MAX_REGISTRY_MODULES, MAX_SEALED_VALUE_BYTES, ModuleBuilder, ModuleError,
    ModuleIdentity, ModuleKind, ModuleRef, ModuleRefError, ModuleRegistry, ModuleSpec, NameError,
    PathError,
};
pub use rejection::{InvalidReason, Rejection, RejectionClass};
pub use resolve::{OciResolver, ResolveError, ResolvedImage, TestResolver, resolve, resolve_with};
pub use revision::TemplateRevision;
pub use schema::{
    Command, DEFAULT_USER, DEFAULT_WORKING_DIRECTORY, EgressIntent, EnvironmentEntry, IdleAction,
    IngressIntent, Lifecycle, MAX_ARGUMENTS, MAX_CIDRS, MAX_DOCUMENT_BYTES, MAX_DOMAINS,
    MAX_ENVIRONMENT, MAX_MODULES, MAX_NAME_BYTES, MAX_SECRETS, MAX_STRING_BYTES, Network,
    Resources, SCHEMA, SecretDelivery, SecretReference, Template, Workload, parse_template,
};
pub use validate::{
    BackendCapabilities, EgressEnvelope, FilesystemOracle, MAX_BACKEND_PLATFORMS, NetworkEnvelope,
    OracleError, PolicyCeiling, ResourceLimits, TestFilesystemOracle,
};
