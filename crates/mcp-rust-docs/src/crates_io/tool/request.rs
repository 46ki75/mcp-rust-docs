use serde::{Deserialize, Serialize};

use crate::crates_io::use_case::SearchCratesUseCaseInput;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchCratesRequest {
    /// Search query. Matches against crate name, description and keywords.
    pub query: String,
    /// Max number of results per page (1-100). Defaults to 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u8>,
    /// 1-indexed page number. Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

impl From<SearchCratesRequest> for SearchCratesUseCaseInput {
    fn from(request: SearchCratesRequest) -> Self {
        Self {
            query: request.query,
            per_page: request.per_page,
            page: request.page,
        }
    }
}
