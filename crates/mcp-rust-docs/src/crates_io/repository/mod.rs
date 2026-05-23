pub mod error;
pub mod input;
pub mod output;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::Mutex;

pub use self::error::CratesIoRepositoryError;
pub use self::input::SearchCratesRepositoryInput;
pub use self::output::{RepositoryCrateRecord, SearchCratesRepositoryOutput};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub type SearchCratesResult = Result<SearchCratesRepositoryOutput, CratesIoRepositoryError>;

pub trait CratesIoRepository: Send + Sync + 'static {
    fn search_crates(
        &self,
        input: SearchCratesRepositoryInput,
    ) -> BoxFuture<'_, SearchCratesResult>;
}

pub struct CratesIoRepositoryImpl {
    http: reqwest::Client,
    base_url: Arc<str>,
}

impl CratesIoRepositoryImpl {
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

#[derive(Default)]
pub struct CratesIoRepositoryStub {
    queue: Mutex<Vec<SearchCratesResult>>,
}

impl CratesIoRepositoryStub {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue(&self, result: SearchCratesResult) {
        self.queue.lock().await.push(result);
    }
}

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
