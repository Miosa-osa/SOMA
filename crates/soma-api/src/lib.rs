#![doc = "HTTP access to the SOMA sandbox lifecycle over the portable facade."]
#![forbid(unsafe_code)]

pub mod capability;
pub mod envelope;
pub mod facade;
pub mod failure;
pub mod filesystem;
pub mod handler;
pub mod http;
pub mod local;
pub mod pool;
pub mod report;
pub mod route;
pub mod tenant;
pub mod terminal;
pub mod wire;

pub use capability::MissingCapability;
pub use envelope::{ApiError, ENVELOPE_SCHEMA, Envelope, FailureBody};
pub use facade::{
    CommandOutcome, FileOutcome, LifecycleOutcome, SandboxFacade, SandboxSnapshot, TerminalOutcome,
};
pub use filesystem::{FilesystemBody, FilesystemReport};
pub use handler::handle;
pub use http::{
    request::{Method, Request},
    response::Response,
    server::serve,
};
pub use local::LocalFacade;
pub use pool::{FacadeLease, FacadePool};
pub use route::{FilesystemOperation, Route, TerminalOperation};
pub use tenant::{TENANT_HEADER, TenantId};
pub use terminal::{TerminalBody, TerminalReport};
