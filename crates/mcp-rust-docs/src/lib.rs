use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};

pub const CRATES_IO_BASE_URL: &str = "https://crates.io";

pub const DEFAULT_USER_AGENT: &str = concat!(
    "mcp-rust-docs/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/46ki75/mcp-rust-docs)",
);

#[derive(Clone)]
pub struct Server {
    #[allow(dead_code)]
    tool_router: ToolRouter<Server>,
    http: reqwest::Client,
    base_url: Arc<str>,
}

impl Server {
    pub fn new() -> anyhow::Result<Self> {
        Self::builder().build()
    }

    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }
}

pub struct ServerBuilder {
    base_url: String,
    user_agent: String,
    http: Option<reqwest::Client>,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            base_url: CRATES_IO_BASE_URL.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            http: None,
        }
    }
}

impl ServerBuilder {
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    pub fn build(self) -> anyhow::Result<Server> {
        let http = match self.http {
            Some(client) => client,
            None => reqwest::Client::builder()
                .user_agent(self.user_agent)
                .build()?,
        };

        Ok(Server {
            tool_router: Server::tool_router(),
            http,
            base_url: Arc::from(self.base_url),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchCratesArgs {
    /// Search query. Matches against crate name, description and keywords.
    pub query: String,
    /// Max number of results per page (1-100). Defaults to 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u8>,
    /// 1-indexed page number. Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CratesIoResponse {
    crates: Vec<RawCrate>,
    meta: Meta,
}

#[derive(Debug, Deserialize)]
struct RawCrate {
    name: String,
    #[serde(default)]
    description: Option<String>,
    max_version: String,
    #[serde(default)]
    max_stable_version: Option<String>,
    downloads: u64,
    #[serde(default)]
    recent_downloads: Option<u64>,
    #[serde(default)]
    documentation: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct Meta {
    total: u64,
}

#[derive(Debug, Serialize)]
struct SearchCratesOutput {
    total: u64,
    page: u32,
    per_page: u8,
    crates: Vec<CrateSummary>,
}

#[derive(Debug, Serialize)]
struct CrateSummary {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    downloads: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    recent_downloads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    updated_at: String,
}

#[tool_router]
impl Server {
    #[tool(
        description = "Search crates on crates.io. Returns name, version, description, download counts and links for each matched crate."
    )]
    async fn search_crates(
        &self,
        Parameters(args): Parameters<SearchCratesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let per_page = args.per_page.unwrap_or(10).clamp(1, 100);
        let page = args.page.unwrap_or(1).max(1);

        let url = format!("{}/api/v1/crates", self.base_url);
        let per_page_str = per_page.to_string();
        let page_str = page.to_string();

        let response = match self
            .http
            .get(&url)
            .query(&[
                ("q", args.query.as_str()),
                ("per_page", per_page_str.as_str()),
                ("page", page_str.as_str()),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(error = %err, "crates.io request failed");
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Request to crates.io failed: {err}"
                ))]));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "crates.io returned HTTP {status}: {body}"
            ))]));
        }

        let parsed: CratesIoResponse = match response.json().await {
            Ok(parsed) => parsed,
            Err(err) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Failed to parse crates.io response: {err}"
                ))]));
            }
        };

        let output = SearchCratesOutput {
            total: parsed.meta.total,
            page,
            per_page,
            crates: parsed
                .crates
                .into_iter()
                .map(|c| CrateSummary {
                    version: c.max_stable_version.unwrap_or(c.max_version),
                    name: c.name,
                    description: c.description,
                    downloads: c.downloads,
                    recent_downloads: c.recent_downloads,
                    documentation: c.documentation,
                    homepage: c.homepage,
                    repository: c.repository,
                    updated_at: c.updated_at,
                })
                .collect(),
        };

        let text = serde_json::to_string_pretty(&output).map_err(|err| {
            McpError::internal_error(format!("failed to serialize tool output: {err}"), None)
        })?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }
}

#[tool_handler]
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
