mod destroy;
mod doctor;
mod exec;
mod file;
mod inspect;
mod launch;
mod list;
mod run;
mod stop;
mod terminal;

use crate::ToolRuntime;
use rmcp::handler::server::router::tool::ToolRouter;

use super::SomaMcpServer;

pub(super) fn router<R: ToolRuntime>() -> ToolRouter<SomaMcpServer<R>> {
    SomaMcpServer::<R>::doctor_router()
        + SomaMcpServer::<R>::run_router()
        + SomaMcpServer::<R>::launch_router()
        + SomaMcpServer::<R>::exec_router()
        + SomaMcpServer::<R>::file_router()
        + SomaMcpServer::<R>::terminal_router()
        + SomaMcpServer::<R>::list_router()
        + SomaMcpServer::<R>::inspect_router()
        + SomaMcpServer::<R>::stop_router()
        + SomaMcpServer::<R>::destroy_router()
}
