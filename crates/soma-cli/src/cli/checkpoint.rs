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

    /// The private checkpoint directory on the configured encrypted mount.
    #[arg(long, value_name = "PATH")]
    pub bundle: std::path::PathBuf,
}
