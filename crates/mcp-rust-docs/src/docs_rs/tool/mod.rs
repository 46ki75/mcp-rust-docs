/// Tool-layer error type and its formatting into [`CallToolResult`].
pub mod error;
/// JSON-Schema-derived request types accepted by the MCP tools.
pub mod request;
/// Serializable response types returned in tool output.
pub mod response;

pub use self::error::DocsRsToolError;
pub use self::request::{GetCrateDocsRequest, SearchCrateSymbolsRequest};
pub use self::response::{GetCrateDocsResponse, SearchCrateSymbolsResponse, SymbolDto};

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::Server;

#[tool_router(router = docs_rs_tool_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Fetch a Rust documentation page from docs.rs and return it as Markdown. Use `crate_name` (required), `version` (optional, defaults to `latest`), and `path` (optional URL tail under the crate's docs root, e.g. `task/struct.JoinHandle.html` or `sync/index.html`)."
    )]
    pub(crate) async fn get_crate_docs(
        &self,
        Parameters(args): Parameters<GetCrateDocsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let output = match self.docs_rs_use_case().fetch_crate_docs(args.into()).await {
            Ok(output) => output,
            Err(err) => return Ok(DocsRsToolError::from(err).into_tool_result()),
        };

        let response = GetCrateDocsResponse::from(output);

        match serde_json::to_string_pretty(&response) {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Err(McpError::internal_error(
                format!("failed to serialize tool output: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Search a Rust crate's public symbols (types, traits, functions, macros, modules, etc.) by name. Returns matched items with their kind, qualified name and the `path` argument that `get_crate_docs` accepts. `query` is a case-insensitive substring match against the qualified name; omit it to list everything. `kinds` filters by kind (`struct`, `enum`, `trait`, `fn`, `macro`, `derive`, `attribute`, `type`, `module`, `constant`, `static`, `union`, `primitive`). `limit` caps results (default 50, max 500)."
    )]
    pub(crate) async fn search_crate_symbols(
        &self,
        Parameters(args): Parameters<SearchCrateSymbolsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let output = match self
            .docs_rs_use_case()
            .search_crate_symbols(args.into())
            .await
        {
            Ok(output) => output,
            Err(err) => return Ok(DocsRsToolError::from(err).into_tool_result()),
        };

        let response = SearchCrateSymbolsResponse::from(output);

        match serde_json::to_string_pretty(&response) {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Err(McpError::internal_error(
                format!("failed to serialize tool output: {err}"),
                None,
            )),
        }
    }
}
