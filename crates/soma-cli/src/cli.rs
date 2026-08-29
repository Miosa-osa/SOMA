use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

mod network;

pub use network::{DnsInput, EgressInput, NetworkArgs, ProtocolInput};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum BackendSelection {
    #[default]
    Auto,
    Macos,
    Kvm,
}

impl From<BackendSelection> for soma_local::BackendSelection {
    fn from(value: BackendSelection) -> Self {
        match value {
            BackendSelection::Auto => Self::Auto,
            BackendSelection::Macos => Self::Macos,
            BackendSelection::Kvm => Self::Kvm,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "soma",
    about = "Run and control hardware-isolated SOMA sandboxes"
)]
pub struct Cli {
    /// Output for people or the stable soma.cli.v1 machine envelope.
    #[arg(long, global = true, value_enum, default_value = "human")]
    pub format: OutputFormat,

    /// Runtime backend. Auto selects the supported local isolation engine.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub backend: BackendSelection,

    /// Explicit Apple container executable. This always wins over discovery.
    #[arg(long, global = true, value_name = "PATH")]
    pub runtime: Option<PathBuf>,

    /// Explicit durable local state root shared with soma-mcp.
    #[arg(long, global = true, value_name = "PATH")]
    pub state_root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: RootCommand,
}

#[derive(Subcommand)]
pub enum RootCommand {
    /// Run one bounded command from an OCI image and prove cleanup.
    Run(RunArgs),
    /// Control one durable sandbox by its exact instance identity.
    Machine(MachineArgs),
    /// Probe the selected backend without overstating production readiness.
    Doctor(DoctorArgs),
    /// Print the SOMA command-line contract version.
    Version,
}

#[derive(Args)]
pub struct MachineArgs {
    #[command(subcommand)]
    pub command: MachineCommand,
}

#[derive(Subcommand)]
pub enum MachineCommand {
    /// Create and start an OCI-backed sandbox.
    Launch(LaunchArgs),
    /// Execute a bounded command in a running sandbox.
    Exec(ExecArgs),
    /// Inspect the portable state of a sandbox.
    Inspect(ControlArgs),
    /// Gracefully stop and release a sandbox.
    Stop(ControlArgs),
    /// Force-destroy a sandbox and release its runtime state.
    Destroy(ControlArgs),
}

#[derive(Args)]
pub struct IdentityArgs {
    /// Caller-selected idempotency identity. A `UUIDv4` simple value is generated when omitted.
    #[arg(long, value_name = "ID")]
    pub operation_id: Option<String>,

    /// Caller-selected sandbox identity. A `UUIDv4` simple value is generated when omitted.
    #[arg(long, value_name = "ID")]
    pub instance_id: Option<String>,
}

#[derive(Clone, Args)]
pub struct ShapeArgs {
    #[arg(long, default_value_t = soma::MachineShape::DEFAULT_VCPU_COUNT)]
    pub vcpus: u16,

    #[arg(long, default_value_t = soma::MachineShape::DEFAULT_MEMORY_MIB)]
    pub memory_mib: u64,

    #[arg(long, default_value_t = soma::MachineShape::DEFAULT_STORAGE_MIB)]
    pub storage_mib: u64,

    #[command(flatten)]
    pub network: NetworkArgs,
}

#[derive(Clone, Args)]
pub struct ExecutionLimitArgs {
    #[arg(long, default_value_t = soma::ExecutionLimits::DEFAULT_TIMEOUT_MS)]
    pub timeout_ms: u64,

    #[arg(long = "max-output-bytes", default_value_t = soma::ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES)]
    pub max_output_bytes: u64,
}

#[derive(Args)]
pub struct RunArgs {
    #[command(flatten)]
    pub identity: IdentityArgs,

    /// Optional lowercase metadata name. It never replaces the instance identity.
    #[arg(long = "name", value_name = "NAME")]
    pub machine_name: Option<String>,

    #[command(flatten)]
    pub shape: ShapeArgs,

    #[command(flatten)]
    pub limits: ExecutionLimitArgs,

    /// OCI image reference.
    pub image: String,

    /// Direct command beginning with an absolute guest executable.
    #[arg(last = true, required = true, num_args = 1..)]
    pub command: Vec<String>,
}

#[derive(Args)]
pub struct LaunchArgs {
    #[command(flatten)]
    pub identity: IdentityArgs,

    /// Optional lowercase metadata name. It never replaces the instance identity.
    #[arg(long = "name", value_name = "NAME")]
    pub machine_name: Option<String>,

    #[command(flatten)]
    pub shape: ShapeArgs,

    /// OCI image reference.
    pub image: String,
}

#[derive(Args)]
pub struct ExecArgs {
    /// Caller-selected idempotency identity. A `UUIDv4` simple value is generated when omitted.
    #[arg(long, value_name = "ID")]
    pub operation_id: Option<String>,

    /// Exact 32-character lowercase sandbox identity.
    #[arg(long, value_name = "ID")]
    pub instance_id: String,

    #[command(flatten)]
    pub limits: ExecutionLimitArgs,

    /// Direct command beginning with an absolute guest executable.
    #[arg(last = true, required = true, num_args = 1..)]
    pub command: Vec<String>,
}

#[derive(Args)]
pub struct ControlArgs {
    /// Caller-selected idempotency identity. A `UUIDv4` simple value is generated when omitted.
    #[arg(long, value_name = "ID")]
    pub operation_id: Option<String>,

    /// Exact 32-character lowercase sandbox identity.
    #[arg(long, value_name = "ID")]
    pub instance_id: String,
}

#[derive(Args)]
pub struct DoctorArgs {
    /// Return a nonzero status unless the selected backend probe passes.
    #[arg(long)]
    pub strict: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{BackendSelection, Cli, EgressInput, MachineCommand, OutputFormat, RootCommand};

    const INSTANCE_ID: &str = "22222222222222222222222222222222";

    #[test]
    fn parses_one_shot_shape_network_and_exact_argv() {
        let cli = Cli::try_parse_from([
            "soma",
            "run",
            "--vcpus",
            "2",
            "--memory-mib",
            "2048",
            "--network",
            "denied",
            "node:22",
            "--",
            "/usr/local/bin/node",
            "--version",
        ])
        .expect("run syntax");
        let RootCommand::Run(run) = cli.command else {
            panic!("run command");
        };
        assert_eq!(run.shape.vcpus, 2);
        assert_eq!(run.shape.memory_mib, 2_048);
        assert_eq!(run.shape.network.egress, EgressInput::Denied);
        assert_eq!(run.command, ["/usr/local/bin/node", "--version"]);
    }

    #[test]
    fn global_runtime_controls_are_accepted_after_nested_subcommands() {
        let cli = Cli::try_parse_from([
            "soma",
            "machine",
            "exec",
            "--format",
            "json",
            "--backend",
            "macos",
            "--state-root",
            "/tmp/soma-state",
            "--instance-id",
            INSTANCE_ID,
            "--",
            "/bin/true",
        ])
        .expect("managed exec syntax");
        assert_eq!(cli.format, OutputFormat::Json);
        assert_eq!(cli.backend, BackendSelection::Macos);
        let RootCommand::Machine(machine) = cli.command else {
            panic!("machine command");
        };
        assert!(matches!(machine.command, MachineCommand::Exec(_)));
    }

    #[test]
    fn removed_control_timeouts_and_start_command_are_rejected() {
        assert!(
            Cli::try_parse_from([
                "soma",
                "machine",
                "stop",
                "--instance-id",
                INSTANCE_ID,
                "--timeout-ms",
                "1",
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["soma", "machine", "start"]).is_err());
    }
}
