use serde::Serialize;

use crate::crates_io::use_case::{CrateSummary, SearchCratesUseCaseOutput};

/// JSON body returned by the `search_crates` tool.
///
/// Mirrors [`SearchCratesUseCaseOutput`] but with the serde
/// annotations needed to keep the on-wire shape stable.
#[derive(Debug, Serialize)]
pub struct SearchCratesResponse {
    /// Total matches across all pages.
    pub total: u64,
    /// Effective 1-indexed page returned.
    pub page: u32,
    /// Effective page size returned.
    pub per_page: u8,
    /// Hits on this page.
    pub crates: Vec<CrateSummaryDto>,
}

/// Wire representation of a single crate hit.
///
/// Optional fields are dropped from the JSON output rather than
/// serialized as `null`, so the model sees only fields that have
/// real values.
#[derive(Debug, Serialize)]
pub struct CrateSummaryDto {
    /// Crate name.
    pub name: String,
    /// Selected version (stable when available, otherwise latest).
    pub version: String,
    /// Crate description, omitted from the JSON when missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Lifetime download count.
    pub downloads: u64,
    /// Downloads in the last 90 days, omitted when missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_downloads: Option<u64>,
    /// Documentation URL, omitted when missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    /// Homepage URL, omitted when missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Source repository URL, omitted when missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// RFC 3339 timestamp of the most recent publish.
    pub updated_at: String,
}

impl From<SearchCratesUseCaseOutput> for SearchCratesResponse {
    fn from(output: SearchCratesUseCaseOutput) -> Self {
        Self {
            total: output.total,
            page: output.page,
            per_page: output.per_page,
            crates: output
                .crates
                .into_iter()
                .map(CrateSummaryDto::from)
                .collect(),
        }
    }
}

impl From<CrateSummary> for CrateSummaryDto {
    fn from(summary: CrateSummary) -> Self {
        Self {
            name: summary.name,
            version: summary.version,
            description: summary.description,
            downloads: summary.downloads,
            recent_downloads: summary.recent_downloads,
            documentation: summary.documentation,
            homepage: summary.homepage,
            repository: summary.repository,
            updated_at: summary.updated_at,
        }
    }
}
