//! The five terminal subcommands and the arguments they take.
//!
//! Each is one call against a session that lives in the sandbox, so five separate processes can
//! drive one terminal: the first opens it, later ones type at it and read from it, and the last
//! closes it. Nothing here holds the session, which is why this works from a shell script.

use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args)]
pub struct PtyArgs {
    #[command(subcommand)]
    pub command: PtyCommand,
}

/// The five terminal operations, one guest operation each.
#[derive(Subcommand)]
pub enum PtyCommand {
    /// Open the sandbox's terminal at the given dimensions.
    Open(PtySizeArgs),
    /// Type bytes at the terminal.
    Write(PtyWriteArgs),
    /// Read one bounded chunk of whatever the terminal has produced.
    Read(PtyReadArgs),
    /// Tell the terminal it has new dimensions.
    Resize(PtySizeArgs),
    /// End the session and everything running under it.
    Close(PtyTargetArgs),
}

/// What every terminal operation needs.
#[derive(Args)]
pub struct PtyTargetArgs {
    /// Caller-selected idempotency identity. A `UUIDv4` simple value is generated when omitted.
    #[arg(long, value_name = "ID")]
    pub operation_id: Option<String>,

    /// Exact 32-character lowercase sandbox identity.
    #[arg(long, value_name = "ID")]
    pub instance_id: String,
}

#[derive(Args)]
pub struct PtySizeArgs {
    #[command(flatten)]
    pub target: PtyTargetArgs,

    /// Width in character cells.
    #[arg(long, default_value_t = 80)]
    pub columns: u16,

    /// Height in character cells.
    #[arg(long, default_value_t = 24)]
    pub rows: u16,
}

#[derive(Args)]
pub struct PtyWriteArgs {
    #[command(flatten)]
    pub target: PtyTargetArgs,

    /// Host file whose exact bytes are typed at the terminal.
    ///
    /// The input comes from a file rather than from an argument so that a write is byte-exact:
    /// what is typed at a terminal includes control characters and escape sequences that an
    /// argument cannot carry. `--input-file /dev/stdin` types what is piped in.
    #[arg(long = "input-file", value_name = "PATH")]
    pub input_file: PathBuf,
}

#[derive(Args)]
pub struct PtyReadArgs {
    #[command(flatten)]
    pub target: PtyTargetArgs,

    /// Longest to wait for the terminal's first byte, in milliseconds.
    #[arg(long = "wait-ms", default_value_t = 0)]
    pub wait_ms: u32,
}
