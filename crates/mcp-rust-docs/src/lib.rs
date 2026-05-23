pub mod crates_io;
pub mod error;
pub mod router;

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool_handler,
};

pub use crate::crates_io::use_case::CratesIoUseCase;
pub use crate::error::Error;
pub use crate::router::{CRATES_IO_BASE_URL, DEFAULT_USER_AGENT, ServerBuilder};

#[derive(Clone)]
pub struct Server {
    #[allow(dead_code)]
    tool_router: ToolRouter<Server>,
    crates_io_use_case: Arc<CratesIoUseCase>,
}

impl Server {
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }

    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    pub fn with_use_case(use_case: Arc<CratesIoUseCase>) -> Self {
        Self {
            tool_router: Self::crates_io_tool_router(),
            crates_io_use_case: use_case,
        }
    }

    pub(crate) fn crates_io_use_case(&self) -> &CratesIoUseCase {
        &self.crates_io_use_case
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Search the crates.io registry. Use the `search_crates` tool with a query string."
                .to_string(),
        );
        info
    }
}
