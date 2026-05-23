use crate::crates_io::repository::CratesIoRepositoryError;

/// Failures returned by the use case.
///
/// `InvalidQuery` is a caller mistake (user input the use case can
/// reject without ever hitting the network). `Repository` wraps the
/// underlying HTTP-tier failure transparently — the tool layer's
/// formatter switches on this distinction to give the model a
/// "fix-your-input" vs "upstream is broken" message.
#[derive(Debug, thiserror::Error)]
pub enum CratesIoUseCaseError {
    /// The request failed validation before any HTTP call was made —
    /// e.g. empty query.
    #[error("invalid search query: {0}")]
    InvalidQuery(String),

    /// The repository call itself failed; see
    /// [`CratesIoRepositoryError`].
    #[error(transparent)]
    Repository(#[from] CratesIoRepositoryError),
}
