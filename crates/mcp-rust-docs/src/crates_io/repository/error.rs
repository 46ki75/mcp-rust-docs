#[derive(Debug, thiserror::Error)]
pub enum CratesIoRepositoryError {
    #[error("HTTP request to crates.io failed: {0}")]
    Network(#[from] reqwest::Error),

    #[error("crates.io returned HTTP {status}: {body}")]
    UpstreamStatus {
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("failed to decode crates.io response: {0}")]
    InvalidResponse(serde_json::Error),
}
