pub mod error;
pub mod input;
pub mod output;

use std::sync::Arc;

pub use self::error::CratesIoUseCaseError;
pub use self::input::SearchCratesUseCaseInput;
pub use self::output::{CrateSummary, SearchCratesUseCaseOutput};

use crate::crates_io::repository::{
    CratesIoRepository, RepositoryCrateRecord, SearchCratesRepositoryInput,
    SearchCratesRepositoryOutput,
};

const DEFAULT_PER_PAGE: u8 = 10;
const MAX_PER_PAGE: u8 = 100;
const DEFAULT_PAGE: u32 = 1;

pub struct CratesIoUseCase {
    repository: Arc<dyn CratesIoRepository>,
}

impl CratesIoUseCase {
    pub fn new(repository: Arc<dyn CratesIoRepository>) -> Self {
        Self { repository }
    }

    #[tracing::instrument(skip(self))]
    pub async fn search_crates(
        &self,
        input: SearchCratesUseCaseInput,
    ) -> Result<SearchCratesUseCaseOutput, CratesIoUseCaseError> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(CratesIoUseCaseError::InvalidQuery(
                "query must not be empty".into(),
            ));
        }

        let per_page = input
            .per_page
            .unwrap_or(DEFAULT_PER_PAGE)
            .clamp(1, MAX_PER_PAGE);
        let page = input.page.unwrap_or(DEFAULT_PAGE).max(1);

        let repo_output = self
            .repository
            .search_crates(SearchCratesRepositoryInput {
                query: query.to_string(),
                per_page,
                page,
            })
            .await?;

        Ok(into_use_case_output(repo_output, page, per_page))
    }
}

fn into_use_case_output(
    output: SearchCratesRepositoryOutput,
    page: u32,
    per_page: u8,
) -> SearchCratesUseCaseOutput {
    SearchCratesUseCaseOutput {
        total: output.total,
        page,
        per_page,
        crates: output.crates.into_iter().map(into_summary).collect(),
    }
}

fn into_summary(record: RepositoryCrateRecord) -> CrateSummary {
    CrateSummary {
        version: record.max_stable_version.unwrap_or(record.max_version),
        name: record.name,
        description: record.description,
        downloads: record.downloads,
        recent_downloads: record.recent_downloads,
        documentation: record.documentation,
        homepage: record.homepage,
        repository: record.repository,
        updated_at: record.updated_at,
    }
}
