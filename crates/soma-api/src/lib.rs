#![doc = "HTTP access to the SOMA sandbox lifecycle over the portable facade."]
#![forbid(unsafe_code)]

pub mod admission;
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
pub mod resources;
pub mod route;
pub mod runner;
pub mod tenant;
pub mod terminal;
pub mod tunnel;
pub mod volumes;
pub mod wire;

pub use admission::{CREATE_ADMISSION_ENV, CreateAdmission, CreatePermit};
pub use capability::MissingCapability;
pub use envelope::{ApiError, ENVELOPE_SCHEMA, Envelope, FailureBody};
pub use facade::{
    CommandOutcome, FileOutcome, LifecycleOutcome, ResizeFailure, ResizeOutcome, ResizeRequest,
    SandboxFacade, SandboxSnapshot, TerminalOutcome, TunnelFailure,
};
pub use filesystem::{FilesystemBody, FilesystemReport};
pub use handler::{handle, handle_prewarm};
pub use http::{
    request::{Method, Request},
    response::Response,
    server::{serve, serve_with_tunnels},
};
pub use local::LocalFacade;
pub use pool::{FacadeLease, FacadePool};
pub use route::{FilesystemOperation, Route, TerminalOperation};
pub use tenant::{TENANT_HEADER, TenantId};
pub use terminal::{TerminalBody, TerminalReport};
pub use volumes::{
    HOSTD_SOCKET_ENV, VOLUME_DEVICE, VolumeBody, VolumeClient, VolumeFailure, VolumeOperation,
    VolumeReport, socket_from,
};
