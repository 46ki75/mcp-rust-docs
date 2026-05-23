/// Failures that can happen at the HTTP boundary against docs.rs.
///
/// `Network` is infrastructure (we never reached or never read the
/// response). `NotFound` is a 404 specifically — broken out from the
/// generic `UpstreamStatus` because the use case maps it to a
/// caller-facing "did you typo the crate name / path?" message rather
/// than a generic upstream failure.
#[derive(Debug, thiserror::Error)]
pub enum DocsRsRepositoryError {
    /// `reqwest` could not complete the request — DNS, TLS, connection
    /// reset, body read failure, etc.
    #[error("HTTP request to docs.rs failed: {0}")]
    Network(#[from] reqwest::Error),

    /// docs.rs returned 404. The URL is captured so the caller can
    /// echo it back to the user (the most common cause is a wrong
    /// crate name or item path).
    #[error("docs.rs returned 404 for {url}")]
    NotFound {
        /// URL that returned 404.
        url: String,
    },

    /// docs.rs returned a non-2xx, non-404 response. Body kept for
    /// diagnostics (usually an HTML error page).
    #[error("docs.rs returned HTTP {status} for {url}")]
    UpstreamStatus {
        /// HTTP status code returned by docs.rs.
        status: reqwest::StatusCode,
        /// URL that triggered the failure.
        url: String,
        /// Raw response body, kept for diagnostics.
        body: String,
    },
}
