use clap::{Args, ValueEnum};

#[derive(Args)]
pub struct CheckpointExportArgs {
    /// The authorized machine name recorded in the broker state.
    #[arg(long, value_name = "NAME")]
    pub owner: String,

    /// Which verified capture file to stream.
    #[arg(long, value_enum)]
    pub artifact: ExportArtifact,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ExportArtifact {
    State,
    Memory,
    Overlay,
    Workspace,
}

#[derive(Args)]
pub struct CheckpointImportArgs {
    /// The authorized machine name recorded in the broker state.
    #[arg(long, value_name = "NAME")]
    pub owner: String,

    /// Reserve recovery on a replacement broker after authenticating and fencing the source.
    #[arg(long)]
    pub recover_absent: bool,

    /// Import only the verified workspace for a cold start on the destination.
    #[arg(long, conflicts_with = "recover_absent", requires_all = ["destination_generation", "destination_volume", "operation"])]
    pub cold_workspace: bool,

    /// Certified Generation for the cold start.
    #[arg(long, requires = "cold_workspace")]
    pub destination_generation: Option<String>,

    /// Destination volume ID.
    #[arg(long, requires = "cold_workspace")]
    pub destination_volume: Option<String>,

    /// Destination Instance ID, defaulting to the source Instance.
    #[arg(long, requires = "cold_workspace")]
    pub destination_instance: Option<String>,

    /// Operation ID for the workspace import.
    #[arg(long, requires = "cold_workspace")]
    pub operation: Option<String>,

    /// The private checkpoint directory on the configured encrypted mount.
    #[arg(long, value_name = "PATH")]
    pub bundle: std::path::PathBuf,
}
