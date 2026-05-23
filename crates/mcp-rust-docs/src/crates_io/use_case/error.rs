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
    /// e.g. empty query, or a metadata request naming a version that
    /// doesn't exist on the resolved crate.
    #[error("invalid request: {0}")]
    InvalidQuery(String),

    /// The registry responded successfully but the payload is
    /// internally inconsistent — e.g. `max_stable_version` names a
    /// version that doesn't appear in the `versions[]` list. Surfaces
    /// as "Upstream failure" at the tool boundary since retrying may
    /// resolve it (stale mirror cache, mid-write GC race).
    #[error("inconsistent upstream response: {0}")]
    InconsistentUpstream(String),

    /// The repository call itself failed; see
    /// [`CratesIoRepositoryError`].
    #[error(transparent)]
    Repository(#[from] CratesIoRepositoryError),
}
