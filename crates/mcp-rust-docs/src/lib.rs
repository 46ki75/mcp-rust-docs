//! MCP server exposing Rust ecosystem tools.
//!
//! Currently ships a single tool — `search_crates` — that queries
//! the crates.io registry. A single binary, `mcp-rust-docs`, adapts
//! this library to the two MCP transports an editor host cares about,
//! selected by subcommand:
//!
//! - `mcp-rust-docs stdio` — line-buffered JSON-RPC over stdin/stdout
//! - `mcp-rust-docs http`  — streamable HTTP at `/mcp`
//!
//! Both transports accept `--crates-io-base-url` (env
//! `MCP_CRATES_IO_BASE_URL`) to override the upstream registry,
//! useful for hermetic tests or registry mirrors.
//!
//! Internally the crate follows the repository / use case / tool
//! layering documented in the org-wide Rust standards.

#![deny(missing_docs)]

/// crates.io integration, split into repository / use case / tool layers.
pub mod crates_io;
/// Crate-wide error type used during [`Server`] construction.
pub mod error;
/// [`Server`] builder and the constants it defaults to.
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

/// MCP server handler bundling every tool this crate exposes.
///
/// Cheap to clone — the use case is behind an `Arc` — which is what
/// lets the streamable-HTTP transport hand out a fresh `Server`
/// per session without rebuilding any state.
#[derive(Clone)]
pub struct Server {
    #[allow(dead_code)]
    tool_router: ToolRouter<Server>,
    crates_io_use_case: Arc<CratesIoUseCase>,
}

impl Server {
    /// Build a server backed by a freshly constructed HTTP client
    /// pointed at the public crates.io registry. Equivalent to
    /// `Self::builder().build()`.
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }

    /// Start a [`ServerBuilder`] for customizing the upstream URL,
    /// HTTP client, user agent, or repository implementation.
    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// Wrap a pre-built use case. Useful when callers want to share
    /// one use case (and therefore one HTTP connection pool) across
    /// many `Server` clones, as the HTTP transport binary does.
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
