/// Per-error-variant docs for repository failures.
pub mod error;
/// Input types accepted by the repository.
pub mod input;
/// Output types returned by the repository.
pub mod output;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Deserialize;

pub use self::error::CratesIoRepositoryError;
pub use self::input::SearchCratesRepositoryInput;
pub use self::output::{RepositoryCrateRecord, SearchCratesRepositoryOutput};

/// Boxed future used to keep the repository trait dyn-compatible.
///
/// See the org standards' _Async traits with `Arc<dyn>`_ section for
/// why we hand-roll this instead of using `#[async_trait]`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Convenience alias for the repository's only result shape.
pub type SearchCratesResult = Result<SearchCratesRepositoryOutput, CratesIoRepositoryError>;

/// Repository abstraction over the crates.io search endpoint.
///
/// Held as `Arc<dyn CratesIoRepository>` by the use case so a stub
/// can be swapped in for tests without touching the real HTTP client.
pub trait CratesIoRepository: Send + Sync + 'static {
    /// Issue a search and return the registry's response, projected
    /// onto [`SearchCratesRepositoryOutput`].
    fn search_crates(
        &self,
        input: SearchCratesRepositoryInput,
    ) -> BoxFuture<'_, SearchCratesResult>;
}

/// Real implementation backed by `reqwest`, talking to crates.io
/// (or any compatible registry mirror) over HTTPS.
pub struct CratesIoRepositoryImpl {
    http: reqwest::Client,
    base_url: Arc<str>,
}

impl CratesIoRepositoryImpl {
    /// Wrap a pre-built `reqwest::Client` and the base URL of the
    /// target registry (e.g. `https://crates.io`). The client's
    /// `User-Agent` header is preserved — set it before passing in.
    pub fn new(http: reqwest::Client, base_url: impl Into<Arc<str>>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
        }
    }
}

impl CratesIoRepository for CratesIoRepositoryImpl {
    fn search_crates(
        &self,
        input: SearchCratesRepositoryInput,
    ) -> BoxFuture<'_, SearchCratesResult> {
        Box::pin(async move {
            let url = format!("{}/api/v1/crates", self.base_url);
            let per_page = input.per_page.to_string();
            let page = input.page.to_string();

            let response = self
                .http
                .get(&url)
                .query(&[
                    ("q", input.query.as_str()),
                    ("per_page", per_page.as_str()),
                    ("page", page.as_str()),
                ])
                .send()
                .await?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(CratesIoRepositoryError::UpstreamStatus { status, body });
            }

            let body_bytes = response.bytes().await?;
            let parsed: CratesIoSearchResponse = serde_json::from_slice(&body_bytes)
                .map_err(CratesIoRepositoryError::InvalidResponse)?;

            Ok(SearchCratesRepositoryOutput {
                total: parsed.meta.total,
                crates: parsed
                    .crates
                    .into_iter()
                    .map(|c| RepositoryCrateRecord {
                        name: c.name,
                        max_version: c.max_version,
                        max_stable_version: c.max_stable_version,
                        description: c.description,
                        downloads: c.downloads,
                        recent_downloads: c.recent_downloads,
                        documentation: c.documentation,
                        homepage: c.homepage,
                        repository: c.repository,
                        updated_at: c.updated_at,
                    })
                    .collect(),
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct CratesIoSearchResponse {
    crates: Vec<CratesIoRawCrate>,
    meta: CratesIoMeta,
}

#[derive(Debug, Deserialize)]
struct CratesIoRawCrate {
    name: String,
    #[serde(default)]
    description: Option<String>,
    max_version: String,
    #[serde(default)]
    max_stable_version: Option<String>,
    downloads: u64,
    #[serde(default)]
    recent_downloads: Option<u64>,
    #[serde(default)]
    documentation: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CratesIoMeta {
    total: u64,
}

/// In-memory stub used by unit tests across the crate. Gated on `cfg(test)`
/// so it never ships in release builds and is invisible to integration
/// tests in `tests/` — those should exercise the real repository through
/// a `wiremock`-backed HTTP upstream.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CratesIoRepositoryStub {
    queue: tokio::sync::Mutex<Vec<SearchCratesResult>>,
}

#[cfg(test)]
impl CratesIoRepositoryStub {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn enqueue(&self, result: SearchCratesResult) {
        self.queue.lock().await.push(result);
    }
}

#[cfg(test)]
impl CratesIoRepository for CratesIoRepositoryStub {
    fn search_crates(
        &self,
        _input: SearchCratesRepositoryInput,
    ) -> BoxFuture<'_, SearchCratesResult> {
        Box::pin(async move {
            self.queue.lock().await.pop().unwrap_or_else(|| {
                Ok(SearchCratesRepositoryOutput {
                    total: 0,
                    crates: vec![],
                })
            })
        })
    }
}
