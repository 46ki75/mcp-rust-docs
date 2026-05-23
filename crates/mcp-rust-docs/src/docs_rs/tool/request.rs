use serde::{Deserialize, Serialize};

use crate::docs_rs::use_case::{FetchCrateDocsUseCaseInput, SearchCrateSymbolsUseCaseInput};

/// Arguments for the `get_crate_docs` tool.
///
/// Field docs double as the JSON Schema descriptions surfaced to the
/// MCP client (and therefore to the model) via `schemars`.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetCrateDocsRequest {
    /// Crate name as published on crates.io (e.g. `tokio`,
    /// `tokio-util`, `serde`). Hyphens in the crate name are
    /// automatically translated to underscores in the resulting
    /// docs.rs URL.
    pub crate_name: String,

    /// Version selector. Accepts a concrete semver string
    /// (`1.40.0`), a docs.rs range (`^1`, `1.*`), or the literal
    /// string `latest`. Defaults to `latest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// URL-path tail relative to the crate's documentation root.
    /// Examples: `task/struct.JoinHandle.html`, `sync/index.html`,
    /// `macro.tokio_test.html`. Omit to fetch the crate root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl From<GetCrateDocsRequest> for FetchCrateDocsUseCaseInput {
    fn from(request: GetCrateDocsRequest) -> Self {
        Self {
            crate_name: request.crate_name,
            version: request.version,
            path: request.path,
        }
    }
}

/// Arguments for the `search_crate_symbols` tool.
///
/// Pair this with `get_crate_docs`: the `path` field on each returned
/// symbol can be passed straight through as `get_crate_docs.path` to
/// fetch the documentation page for that item.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchCrateSymbolsRequest {
    /// Crate name as published on crates.io.
    pub crate_name: String,

    /// Version selector. Defaults to `latest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Case-insensitive substring matched against each item's
    /// qualified name (e.g. `de::value::U8Deserializer`). Omit or
    /// pass an empty string to match every item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    /// Optional kind filter. Accepted vocabulary: `struct`, `enum`,
    /// `trait`, `union`, `macro`, `derive`, `attribute`, `fn`,
    /// `type`, `module`, `constant`, `static`, `primitive`.
    /// Case-insensitive. Which kinds actually appear depends on what
    /// the target crate exposes — most crates have only a handful.
    /// Unknown values match nothing; an empty list is treated as
    /// "no filter".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,

    /// Maximum items to return. Defaults to 50, clamped to 500.
    /// `total_matched` in the response always reports the true
    /// count before truncation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl From<SearchCrateSymbolsRequest> for SearchCrateSymbolsUseCaseInput {
    fn from(request: SearchCrateSymbolsRequest) -> Self {
        Self {
            crate_name: request.crate_name,
            version: request.version,
            query: request.query,
            kinds: request.kinds,
            limit: request.limit,
        }
    }
}
