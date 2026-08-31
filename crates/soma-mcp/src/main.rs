use std::path::PathBuf;

use clap::Parser as _;
use rmcp::ServiceExt as _;

#[derive(clap::Parser)]
#[command(
    name = "soma-mcp",
    about = "Serve the bounded SOMA sandbox MCP interface over stdio"
)]
struct Arguments {
    /// Explicit Apple container executable. This always wins over discovery.
    #[arg(long, value_name = "PATH")]
    runtime: Option<PathBuf>,

    /// Explicit durable state root shared with the soma CLI.
    #[arg(long, value_name = "PATH")]
    state_root: Option<PathBuf>,
}

/// The hidden first argument that turns this executable into a machine host.
///
/// A managed launch starts a host by re-executing the binary it is already running, so every
/// executable that can launch a machine has to be able to hold one. Serving it here rather than
/// as a clap subcommand keeps it out of the MCP surface entirely: it is not a tool, it has no
/// schema, and no protocol session ever reaches it.
const MACHINE_HOST: &str = "machine-host";

#[tokio::main]
async fn main() {
    let mut invocation = std::env::args_os().skip(1);
    if invocation.next().as_deref() == Some(std::ffi::OsStr::new(MACHINE_HOST)) {
        let socket = invocation.next().unwrap_or_default();
        std::process::exit(soma_local::host_machine(std::path::Path::new(&socket)));
    }
    let arguments = Arguments::parse();
    let runtime = soma_mcp::LocalToolRuntime::new(arguments.runtime, arguments.state_root);
    let server = soma_mcp::SomaMcpServer::new(runtime);
    let result = server.serve(soma_mcp::bounded_stdio()).await;
    match result {
        Ok(service) => {
            if let Err(error) = service.waiting().await {
                eprintln!("soma-mcp: protocol session failed: {error}");
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("soma-mcp: failed to start stdio session: {error}");
            std::process::exit(1);
        }
    }
}
