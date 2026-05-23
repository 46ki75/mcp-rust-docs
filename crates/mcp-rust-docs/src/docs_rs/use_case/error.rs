use crate::docs_rs::repository::DocsRsRepositoryError;

/// Failures returned by the use case.
///
/// `InvalidInput` is a caller mistake the use case can reject without
/// hitting the network. `Repository` wraps the underlying HTTP-tier
/// failure transparently — the tool layer's formatter switches on
/// this distinction to give the model a "fix-your-input" vs
/// "upstream is broken" message.
#[derive(Debug, thiserror::Error)]
pub enum DocsRsUseCaseError {
    /// The request failed validation before any HTTP call was made —
    /// e.g. empty crate name, or a `path` containing `..`.
    #[error("invalid docs.rs input: {0}")]
    InvalidInput(String),

    /// The repository call itself failed; see
    /// [`DocsRsRepositoryError`].
    #[error(transparent)]
    Repository(#[from] DocsRsRepositoryError),
}
