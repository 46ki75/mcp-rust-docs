//! MCP server exposing Rust ecosystem tools.
//!
//! Currently ships four tools:
//!
//! - `search_crates` — queries the crates.io registry.
//! - `get_crate_docs` — fetches a documentation page from docs.rs
//!   and returns it as Markdown.
//! - `search_crate_symbols` — lists public items in a crate by name,
//!   handing back paths that `get_crate_docs` accepts.
//! - `grep_crate_docs` — full-text searches a crate's doc-comments
//!   (sourced from docs.rs's rustdoc JSON), returning items whose
//!   docs contain the query plus a snippet.
//!
//! A single binary, `mcp-rust-docs`, adapts this library to the two
//! MCP transports an editor host cares about, selected by subcommand:
//!
//! - `mcp-rust-docs stdio` — line-buffered JSON-RPC over stdin/stdout
//! - `mcp-rust-docs http`  — streamable HTTP at `/mcp`
//!
//! Both transports accept `--crates-io-base-url` (env
//! `MCP_CRATES_IO_BASE_URL`) and `--docs-rs-base-url` (env
//! `MCP_DOCS_RS_BASE_URL`) to override the upstreams, useful for
//! hermetic tests or registry mirrors.
//!
//! Internally the crate follows the repository / use case / tool
//! layering documented in the org-wide Rust standards.

#![deny(missing_docs)]

/// crates.io integration, split into repository / use case / tool layers.
pub mod crates_io;
/// docs.rs integration, split into repository / use case / tool layers.
pub mod docs_rs;
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
pub use crate::docs_rs::use_case::DocsRsUseCase;
pub use crate::error::Error;
pub use crate::router::{CRATES_IO_BASE_URL, DEFAULT_USER_AGENT, DOCS_RS_BASE_URL, ServerBuilder};

/// MCP server handler bundling every tool this crate exposes.
///
/// Cheap to clone — both use cases are behind `Arc`s — which is what
/// lets the streamable-HTTP transport hand out a fresh `Server` per
/// session without rebuilding any state.
#[derive(Clone)]
pub struct Server {
    tool_router: ToolRouter<Server>,
    crates_io_use_case: Arc<CratesIoUseCase>,
    docs_rs_use_case: Arc<DocsRsUseCase>,
}

impl Server {
    /// Build a server backed by freshly constructed HTTP clients
    /// pointed at the public crates.io registry and docs.rs.
    /// Equivalent to `Self::builder().build()`.
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }

    /// Start a [`ServerBuilder`] for customizing the upstream URLs,
    /// HTTP client, user agent, or repository implementations.
    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// Wrap pre-built use cases. Useful when callers want to share
    /// one HTTP connection pool across many `Server` clones, as the
    /// HTTP transport binary does.
    pub fn with_use_cases(
        crates_io_use_case: Arc<CratesIoUseCase>,
        docs_rs_use_case: Arc<DocsRsUseCase>,
    ) -> Self {
        // Merge the per-module tool routers. Adding a third tool
        // module means generating a third `#[tool_router]` impl and
        // adding it to this chain — nothing else moves.
        let tool_router = Self::crates_io_tool_router() + Self::docs_rs_tool_router();
        Self {
            tool_router,
            crates_io_use_case,
            docs_rs_use_case,
        }
    }

    pub(crate) fn crates_io_use_case(&self) -> &CratesIoUseCase {
        &self.crates_io_use_case
    }

    pub(crate) fn docs_rs_use_case(&self) -> &DocsRsUseCase {
        &self.docs_rs_use_case
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Rust ecosystem tools. `search_crates` queries the crates.io \
             registry; `search_crate_symbols` lists a crate's public items \
             by name; `grep_crate_docs` full-text-searches a crate's \
             doc-comments via the rustdoc JSON; `get_crate_docs` fetches a \
             page from docs.rs and returns it as Markdown. Typical flow: \
             search_crates → search_crate_symbols / grep_crate_docs → \
             get_crate_docs."
                .to_string(),
        );
        info
    }
}
