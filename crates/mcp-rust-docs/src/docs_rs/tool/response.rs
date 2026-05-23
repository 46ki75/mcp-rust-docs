use serde::Serialize;

use crate::docs_rs::use_case::FetchCrateDocsUseCaseOutput;

/// JSON body returned by the `get_crate_docs` tool.
///
/// Mirrors [`FetchCrateDocsUseCaseOutput`] but with serde annotations
/// that drop missing optional fields rather than emitting `null`.
#[derive(Debug, Serialize)]
pub struct GetCrateDocsResponse {
    /// Crate name as requested.
    pub crate_name: String,

    /// Concrete version docs.rs actually served, parsed out of the
    /// redirected URL. Omitted from the JSON when the URL shape didn't
    /// match the expected `/{crate}/{version}/...` layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_version: Option<String>,

    /// Canonical URL of the page on docs.rs, after redirects.
    pub url: String,

    /// Page contents converted to Markdown. The use case extracts
    /// the `<main>` element from the rustdoc HTML before conversion,
    /// so the sidebar / search box / footer are not included.
    pub markdown: String,
}

impl From<FetchCrateDocsUseCaseOutput> for GetCrateDocsResponse {
    fn from(output: FetchCrateDocsUseCaseOutput) -> Self {
        Self {
            crate_name: output.crate_name,
            resolved_version: output.resolved_version,
            url: output.final_url,
            markdown: output.markdown,
        }
    }
}
