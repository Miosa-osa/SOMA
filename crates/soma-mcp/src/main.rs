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

#[tokio::main]
async fn main() {
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
