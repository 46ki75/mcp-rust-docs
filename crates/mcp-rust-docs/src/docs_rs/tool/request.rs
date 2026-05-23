use serde::{Deserialize, Serialize};

use crate::docs_rs::use_case::FetchCrateDocsUseCaseInput;

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
