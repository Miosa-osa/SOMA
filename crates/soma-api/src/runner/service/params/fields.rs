//! The create body's own vocabularies: what this runner refuses on sight, and every field the
//! platform defines.
//!
//! They are long lists rather than logic, and they change when the platform's create contract
//! changes rather than when this runner's behaviour does, so they are kept where a reader can see
//! the whole vocabulary at once instead of scrolling past it to the code that consults it.

/// Create parameters that ask for something other than the bare prepared sandbox.
///
/// This is the fast-lane door's own list (`Web.SomaFastLane.Handler`'s `@unsupported_params`):
/// on that path these requests fell through to the standard pipeline, and the runner has none.
/// `cwd` is the one addition to that list: contract C2 names a create carrying it among the
/// requests the runner does not serve, and only this list can refuse it with that answer.
pub(super) const UNSUPPORTED_CREATE_PARAMS: [&str; 16] = [
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
pub(super) const CREATE_FIELDS: [&str; 63] = [
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
    "network",
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
    "run",
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
    "wait_ready",
    "workdir",
    "workspace_id",
    "workspace_name",
    "workspace_slug",
];
