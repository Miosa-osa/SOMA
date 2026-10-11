//! The live resize subcommand.

use clap::Args;

#[derive(Args)]
pub struct ResizeArgs {
    /// Exact 32-character lowercase sandbox identity.
    #[arg(long, value_name = "ID")]
    pub instance_id: String,

    /// The vCPUs the sandbox may use, between 1 and the count it booted with.
    #[arg(long, value_name = "N")]
    pub cpu: Option<u16>,

    /// The total memory the sandbox should have, in MiB.
    #[arg(long, value_name = "N")]
    pub memory_mib: Option<u64>,
}
