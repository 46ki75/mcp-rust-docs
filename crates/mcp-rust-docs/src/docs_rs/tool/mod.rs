/// Tool-layer error type and its formatting into [`CallToolResult`].
pub mod error;
/// JSON-Schema-derived request types accepted by the MCP tools.
pub mod request;
/// Serializable response types returned in tool output.
pub mod response;

pub use self::error::DocsRsToolError;
pub use self::request::{GetCrateDocsRequest, SearchCrateDocsRequest, SearchCrateSymbolsRequest};
pub use self::response::{
    CrateVersionDto, DependencyEntryDto, DependencySummaryDto, DocHitDto, GetCrateDocsResponse,
    MetadataDto, SearchCrateDocsResponse, SearchCrateSymbolsResponse, SymbolDto,
};

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::Server;
use crate::crates_io::use_case::GetCrateMetadataUseCaseInput;

#[tool_router(router = docs_rs_tool_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Fetch a Rust documentation page from docs.rs and return it as Markdown. Use `crate_name` (required), `version` (optional, defaults to `latest`), and `path` (optional URL tail under the crate's docs root, e.g. `task/struct.JoinHandle.html` or `sync/index.html`).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    pub(crate) async fn get_crate_docs(
        &self,
        Parameters(args): Parameters<GetCrateDocsRequest>,
    ) -> Result<CallToolResult, McpError> {
        // Crate-root calls (no `path`) attach metadata fetched in
        // parallel with the docs page. Drill-down calls skip the
        // metadata trip — the agent already has it from the root
        // call, and re-fetching per page would be wasteful.
        let want_metadata = args.path.is_none();
        let crate_name = args.crate_name.clone();
        let version = args.version.clone();

        let response = if want_metadata {
            let docs_fut = self.docs_rs_use_case().fetch_crate_docs(args.into());
            let metadata_fut =
                self.crates_io_use_case()
                    .get_crate_metadata(GetCrateMetadataUseCaseInput {
                        crate_name,
                        version,
                    });
            let (docs_result, metadata_result) = tokio::join!(docs_fut, metadata_fut);

            let docs_output = match docs_result {
                Ok(output) => output,
                Err(err) => return Ok(DocsRsToolError::from(err).into_tool_result()),
            };
            // Metadata is best-effort: a crates.io blip must not kill
            // a successful docs fetch. Format the error so the caller
            // sees what happened and can decide to retry.
            let metadata = metadata_result.map_err(|err| err.to_string());
            GetCrateDocsResponse::from_docs_and_metadata(docs_output, metadata)
        } else {
            let docs_output = match self.docs_rs_use_case().fetch_crate_docs(args.into()).await {
                Ok(output) => output,
                Err(err) => return Ok(DocsRsToolError::from(err).into_tool_result()),
            };
            GetCrateDocsResponse::from_docs_only(docs_output)
        };

        match serde_json::to_string_pretty(&response) {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Err(McpError::internal_error(
                format!("failed to serialize tool output: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Search a Rust crate's public symbols (types, traits, functions, macros, modules, etc.) by name. Returns matched items with their kind, qualified name and the `path` argument that `get_crate_docs` accepts. `query` is a case-insensitive substring match against the qualified name; omit it to list everything. `kinds` filters by kind (`struct`, `enum`, `trait`, `fn`, `macro`, `derive`, `attribute`, `type`, `module`, `constant`, `static`, `union`, `primitive`). `limit` caps results (default 50, max 500).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
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

    #[tool(
        description = "Full-text search across a Rust crate's doc-comments. Fetches the crate's rustdoc JSON from docs.rs and returns every documented item whose doc-comment body contains `query` (case-insensitive substring). Each hit carries the item's kind, qualified name, ~200-char snippet, and the `path` argument that `get_crate_docs` accepts. Unlike `search_crate_symbols` (which matches item names only), this searches the body text of the docs. Use when looking for usage notes, concepts, or examples that aren't visible in symbol names — e.g. \"zero-copy\", \"Pin\", \"thread-safe\". `query` is required and non-empty. `kinds` filters as in search_crate_symbols. `limit` defaults to 20, max 100.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    pub(crate) async fn search_crate_docs(
        &self,
        Parameters(args): Parameters<SearchCrateDocsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let output = match self.docs_rs_use_case().search_crate_docs(args.into()).await {
            Ok(output) => output,
            Err(err) => return Ok(DocsRsToolError::from(err).into_tool_result()),
        };

        let response = SearchCrateDocsResponse::from(output);

        match serde_json::to_string_pretty(&response) {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Err(McpError::internal_error(
                format!("failed to serialize tool output: {err}"),
                None,
            )),
        }
    }
}
