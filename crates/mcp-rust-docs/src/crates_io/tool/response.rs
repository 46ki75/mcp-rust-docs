use serde::Serialize;

use crate::crates_io::use_case::{CrateSummary, SearchCratesUseCaseOutput};

#[derive(Debug, Serialize)]
pub struct SearchCratesResponse {
    pub total: u64,
    pub page: u32,
    pub per_page: u8,
    pub crates: Vec<CrateSummaryDto>,
}

#[derive(Debug, Serialize)]
pub struct CrateSummaryDto {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub downloads: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_downloads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
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
