use crate::crates_io::repository::CratesIoRepositoryError;

#[derive(Debug, thiserror::Error)]
pub enum CratesIoUseCaseError {
    #[error("invalid search query: {0}")]
    InvalidQuery(String),

    #[error(transparent)]
    Repository(#[from] CratesIoRepositoryError),
}
