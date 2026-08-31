//! The six filesystem subcommands and the arguments they take.
//!
//! They live apart from the lifecycle commands because they are the only ones whose arguments are
//! not text: a guest path is bytes, and a file's contents are bytes, so both are carried in forms
//! that do not narrow them to what happens to be valid UTF-8.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args)]
pub struct FileArgs {
    #[command(subcommand)]
    pub command: FileCommand,
}

/// The six filesystem operations, one guest operation each.
#[derive(Subcommand)]
pub enum FileCommand {
    /// Read a whole file out of the sandbox.
    Read(FilePathArgs),
    /// Replace a file in the sandbox with the exact bytes of a host file.
    Write(FileWriteArgs),
    /// Create a directory, and any parent it needs.
    Mkdir(FilePathArgs),
    /// List one directory.
    List(FilePathArgs),
    /// Report whether a path exists and what it is.
    Exists(FilePathArgs),
    /// Remove a file, or a directory with `--recursive`.
    Remove(FileRemoveArgs),
}

/// What every filesystem operation needs.
///
/// The path is an `OsString` rather than a `String` because a guest path is bytes and this
/// surface can carry the operating system's own bytes unchanged. Narrowing it to UTF-8 here
/// would make a legal guest path unnameable from the command line.
#[derive(Args)]
pub struct FilePathArgs {
    /// Caller-selected idempotency identity. A `UUIDv4` simple value is generated when omitted.
    #[arg(long, value_name = "ID")]
    pub operation_id: Option<String>,

    /// Exact 32-character lowercase sandbox identity.
    #[arg(long, value_name = "ID")]
    pub instance_id: String,

    /// Absolute path inside the sandbox.
    pub path: OsString,
}

#[derive(Args)]
pub struct FileWriteArgs {
    #[command(flatten)]
    pub target: FilePathArgs,

    /// Host file whose exact bytes become the sandbox file's contents.
    ///
    /// The contents come from a file rather than from an argument so that a write is byte-exact:
    /// an argument would have to be text, and text cannot carry a file that is not valid UTF-8.
    /// Piping through `/dev/stdin` is the way to write bytes that are not already in a file.
    #[arg(long = "content-file", value_name = "PATH")]
    pub content_file: PathBuf,
}

#[derive(Args)]
pub struct FileRemoveArgs {
    #[command(flatten)]
    pub target: FilePathArgs,

    /// Remove a directory together with everything under it.
    #[arg(long)]
    pub recursive: bool,
}
