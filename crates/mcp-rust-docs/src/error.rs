#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}
