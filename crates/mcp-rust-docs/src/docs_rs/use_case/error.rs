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

    /// docs.rs returned 404 for every format-version variant the
    /// fallback chain tried. Surfaced separately from a single
    /// [`DocsRsRepositoryError::NotFound`] so the model can distinguish
    /// "this crate doesn't exist" (most likely a typo) from "this
    /// crate exists but docs.rs hasn't built any rustdoc-JSON format
    /// this tool understands" (a real upstream-vs-tooling gap, often
    /// transient until docs.rs rebuilds).
    #[error(
        "docs.rs has no rustdoc JSON for {crate_name} at any supported format version ({tried:?})"
    )]
    FormatVersionUnavailable {
        /// The crate name the use case was asked about.
        crate_name: String,
        /// Format versions the fallback chain attempted, in order.
        tried: Vec<u32>,
    },
}
