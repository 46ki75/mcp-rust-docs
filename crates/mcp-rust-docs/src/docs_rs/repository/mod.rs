/// Per-error-variant docs for repository failures.
pub mod error;
/// Input types accepted by the repository.
pub mod input;
/// Output types returned by the repository.
pub mod output;

pub use self::error::DocsRsRepositoryError;
pub use self::input::FetchCrateDocsRepositoryInput;
pub use self::output::FetchCrateDocsRepositoryOutput;

use crate::crates_io::repository::BoxFuture;

/// Convenience alias for the repository's only result shape.
pub type FetchCrateDocsResult = Result<FetchCrateDocsRepositoryOutput, DocsRsRepositoryError>;

/// Repository abstraction over docs.rs HTML fetches.
///
/// Held as `Arc<dyn DocsRsRepository>` by the use case so a stub can
/// be swapped in for tests without touching the real HTTP client.
pub trait DocsRsRepository: Send + Sync + 'static {
    /// Fetch the documentation page at the given URL and return the
    /// raw HTML body plus the post-redirect final URL.
    fn fetch_crate_docs(
        &self,
        input: FetchCrateDocsRepositoryInput,
    ) -> BoxFuture<'_, FetchCrateDocsResult>;
}

/// Real implementation backed by `reqwest`, talking to docs.rs (or
/// any compatible mirror) over HTTPS.
pub struct DocsRsRepositoryImpl {
    http: reqwest::Client,
}

impl DocsRsRepositoryImpl {
    /// Wrap a pre-built `reqwest::Client`. The client's `User-Agent`
    /// header is preserved — set it before passing in.
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

impl DocsRsRepository for DocsRsRepositoryImpl {
    fn fetch_crate_docs(
        &self,
        input: FetchCrateDocsRepositoryInput,
    ) -> BoxFuture<'_, FetchCrateDocsResult> {
        Box::pin(async move {
            let response = self.http.get(&input.url).send().await?;
            let final_url = response.url().to_string();
            let status = response.status();

            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(DocsRsRepositoryError::NotFound { url: final_url });
            }

            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(DocsRsRepositoryError::UpstreamStatus {
                    status,
                    url: final_url,
                    body,
                });
            }

            let html = response.text().await?;
            Ok(FetchCrateDocsRepositoryOutput { final_url, html })
        })
    }
}

/// In-memory stub used by unit tests across the crate. Gated on
/// `cfg(test)` so it never ships in release builds and is invisible
/// to integration tests in `tests/`.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct DocsRsRepositoryStub {
    queue: tokio::sync::Mutex<Vec<FetchCrateDocsResult>>,
    /// Captured URLs so unit tests can assert the use case built the
    /// right docs.rs URL.
    seen: tokio::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl DocsRsRepositoryStub {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn enqueue(&self, result: FetchCrateDocsResult) {
        self.queue.lock().await.push(result);
    }

    pub(crate) async fn last_seen_url(&self) -> Option<String> {
        self.seen.lock().await.last().cloned()
    }
}

#[cfg(test)]
impl DocsRsRepository for DocsRsRepositoryStub {
    fn fetch_crate_docs(
        &self,
        input: FetchCrateDocsRepositoryInput,
    ) -> BoxFuture<'_, FetchCrateDocsResult> {
        Box::pin(async move {
            self.seen.lock().await.push(input.url.clone());
            self.queue.lock().await.pop().unwrap_or_else(|| {
                Ok(FetchCrateDocsRepositoryOutput {
                    final_url: input.url,
                    html: String::new(),
                })
            })
        })
    }
}
