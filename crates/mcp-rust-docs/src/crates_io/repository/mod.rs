pub mod error;
pub mod input;
pub mod output;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::Mutex;

pub use self::error::CratesIoRepositoryError;
pub use self::input::SearchCratesRepositoryInput;
pub use self::output::{RepositoryCrateRecord, SearchCratesRepositoryOutput};

#[async_trait]
pub trait CratesIoRepository: Send + Sync + 'static {
    async fn search_crates(
        &self,
        input: SearchCratesRepositoryInput,
    ) -> Result<SearchCratesRepositoryOutput, CratesIoRepositoryError>;
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

#[async_trait]
impl CratesIoRepository for CratesIoRepositoryImpl {
    async fn search_crates(
        &self,
        input: SearchCratesRepositoryInput,
    ) -> Result<SearchCratesRepositoryOutput, CratesIoRepositoryError> {
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
    queue: Mutex<Vec<Result<SearchCratesRepositoryOutput, CratesIoRepositoryError>>>,
}

impl CratesIoRepositoryStub {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue(
        &self,
        result: Result<SearchCratesRepositoryOutput, CratesIoRepositoryError>,
    ) {
        self.queue.lock().await.push(result);
    }
}

#[async_trait]
impl CratesIoRepository for CratesIoRepositoryStub {
    async fn search_crates(
        &self,
        _input: SearchCratesRepositoryInput,
    ) -> Result<SearchCratesRepositoryOutput, CratesIoRepositoryError> {
        self.queue.lock().await.pop().unwrap_or_else(|| {
            Ok(SearchCratesRepositoryOutput {
                total: 0,
                crates: vec![],
            })
        })
    }
}
