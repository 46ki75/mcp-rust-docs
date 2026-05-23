/// Errors raised when constructing a [`Server`][crate::Server].
///
/// Runtime tool-call failures are reported separately by the per-layer
/// error enums under [`crates_io`][crate::crates_io].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The underlying `reqwest::Client` could not be built — usually a
    /// TLS-backend or system-configuration problem.
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}
