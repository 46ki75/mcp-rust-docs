/// Failures that can happen at the HTTP boundary against crates.io.
///
/// `Network` and `InvalidResponse` are infrastructure failures (we
/// never reached or never understood the registry). `UpstreamStatus`
/// is the registry itself rejecting the request — the body is kept
/// for diagnostics, since crates.io sometimes returns JSON errors and
/// sometimes returns plain text depending on the status.
#[derive(Debug, thiserror::Error)]
pub enum CratesIoRepositoryError {
    /// `reqwest` could not complete the request — DNS, TLS, connection
    /// reset, body read failure, etc.
    #[error("HTTP request to crates.io failed: {0}")]
    Network(#[from] reqwest::Error),

    /// The registry returned a non-2xx response. Body is captured
    /// verbatim — may be JSON or plain text.
    #[error("crates.io returned HTTP {status} for {url}: {body}")]
    UpstreamStatus {
        /// HTTP status code returned by the registry.
        status: reqwest::StatusCode,
        /// URL that triggered the failure. Mirrors the same field on
        /// `DocsRsRepositoryError::UpstreamStatus` so operators get
        /// matching diagnostics from both upstreams.
        url: String,
        /// Raw response body, kept for diagnostics.
        body: String,
    },

    /// The response was 2xx but didn't deserialize against the search
    /// schema — usually means the registry changed its response shape.
    #[error("failed to decode crates.io response: {0}")]
    InvalidResponse(serde_json::Error),
}
