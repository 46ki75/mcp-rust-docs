pub mod error;
pub mod request;
pub mod response;

pub use self::error::CratesIoToolError;
pub use self::request::SearchCratesRequest;
pub use self::response::{CrateSummaryDto, SearchCratesResponse};

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::Server;

#[tool_router(router = crates_io_tool_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Search crates on crates.io. Returns name, version, description, download counts and links for each matched crate."
    )]
    pub(crate) async fn search_crates(
        &self,
        Parameters(args): Parameters<SearchCratesRequest>,
    ) -> Result<CallToolResult, McpError> {
        let output = match self.crates_io_use_case().search_crates(args.into()).await {
            Ok(output) => output,
            Err(err) => return Ok(CratesIoToolError::from(err).into_tool_result()),
        };

        let response = SearchCratesResponse::from(output);

        match serde_json::to_string_pretty(&response) {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Err(McpError::internal_error(
                format!("failed to serialize tool output: {err}"),
                None,
            )),
        }
    }
}
