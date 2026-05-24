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
pub use self::input::{
    FetchCrateInput, FetchCrateVersionDependenciesInput, SearchCratesRepositoryInput,
};
pub use self::output::{
    FetchCrateRepositoryOutput, FetchCrateVersionDependenciesRepositoryOutput,
    RepositoryCrateRecord, RepositoryCrateVersion, RepositoryDependency, RepositoryDependencyKind,
    SearchCratesRepositoryOutput,
};

/// Boxed future used to keep the repository trait dyn-compatible.
///
/// See the org standards' _Async traits with `Arc<dyn>`_ section for
/// why we hand-roll this instead of using `#[async_trait]`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Convenience alias for the search endpoint result.
pub type SearchCratesResult = Result<SearchCratesRepositoryOutput, CratesIoRepositoryError>;

/// Convenience alias for the per-crate fetch result.
pub type FetchCrateResult = Result<FetchCrateRepositoryOutput, CratesIoRepositoryError>;

/// Convenience alias for the dependencies fetch result.
pub type FetchCrateVersionDependenciesResult =
    Result<FetchCrateVersionDependenciesRepositoryOutput, CratesIoRepositoryError>;

/// Repository abstraction over the crates.io HTTP API.
///
/// Held as `Arc<dyn CratesIoRepository>` by the use case so a stub
/// can be swapped in for tests without touching the real HTTP client.
/// The three methods correspond to the three endpoints we hit:
/// search, per-crate aggregate, per-version dependencies.
pub trait CratesIoRepository: Send + Sync + 'static {
    /// Issue a search and return the registry's response, projected
    /// onto [`SearchCratesRepositoryOutput`].
    fn search_crates(
        &self,
        input: SearchCratesRepositoryInput,
    ) -> BoxFuture<'_, SearchCratesResult>;

    /// Fetch the aggregate per-crate record: versions list with
    /// features, `max_version` / `max_stable_version`. 404 surfaces
    /// as [`CratesIoRepositoryError::NotFound`].
    fn fetch_crate(&self, input: FetchCrateInput) -> BoxFuture<'_, FetchCrateResult>;

    /// Fetch the dependency list for a specific published version.
    /// 404 surfaces as [`CratesIoRepositoryError::NotFound`] — either
    /// the crate or the version is unknown.
    fn fetch_crate_version_dependencies(
        &self,
        input: FetchCrateVersionDependenciesInput,
    ) -> BoxFuture<'_, FetchCrateVersionDependenciesResult>;
}

/// Real implementation backed by `reqwest`, talking to crates.io
/// (or any compatible registry mirror) over HTTPS.
pub struct CratesIoRepositoryImpl {
    http: reqwest::Client,
    base_url: Arc<str>,
    max_body_bytes: usize,
}

impl CratesIoRepositoryImpl {
    /// Wrap a pre-built `reqwest::Client` and the base URL of the
    /// target registry (e.g. `https://crates.io`). The client's
    /// `User-Agent` header is preserved — set it before passing in.
    pub fn new(http: reqwest::Client, base_url: impl Into<Arc<str>>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            max_body_bytes: crate::router::DEFAULT_UPSTREAM_BODY_BYTES,
        }
    }

    /// Override the per-response body-size cap. Below this, body bytes
    /// are streamed into memory normally; above, the request errors
    /// with [`CratesIoRepositoryError::PayloadTooLarge`].
    pub fn with_max_body_bytes(mut self, limit: usize) -> Self {
        self.max_body_bytes = limit;
        self
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

            let body_bytes =
                check_status_and_read_body(response, &url, self.max_body_bytes).await?;
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

    fn fetch_crate(&self, input: FetchCrateInput) -> BoxFuture<'_, FetchCrateResult> {
        Box::pin(async move {
            let url = format!("{}/api/v1/crates/{}", self.base_url, input.crate_name);
            let response = self.http.get(&url).send().await?;
            let body_bytes =
                check_status_and_read_body(response, &url, self.max_body_bytes).await?;
            let parsed: CratesIoCrateResponse = serde_json::from_slice(&body_bytes)
                .map_err(CratesIoRepositoryError::InvalidResponse)?;

            Ok(FetchCrateRepositoryOutput {
                name: parsed.krate.name,
                max_version: parsed.krate.max_version,
                max_stable_version: parsed.krate.max_stable_version,
                versions: parsed
                    .versions
                    .into_iter()
                    .map(|v| RepositoryCrateVersion {
                        num: v.num,
                        yanked: v.yanked,
                        created_at: v.created_at,
                        features: v.features,
                    })
                    .collect(),
            })
        })
    }

    fn fetch_crate_version_dependencies(
        &self,
        input: FetchCrateVersionDependenciesInput,
    ) -> BoxFuture<'_, FetchCrateVersionDependenciesResult> {
        Box::pin(async move {
            let url = format!(
                "{}/api/v1/crates/{}/{}/dependencies",
                self.base_url, input.crate_name, input.version,
            );
            let response = self.http.get(&url).send().await?;
            let body_bytes =
                check_status_and_read_body(response, &url, self.max_body_bytes).await?;
            let parsed: CratesIoDependenciesResponse = serde_json::from_slice(&body_bytes)
                .map_err(CratesIoRepositoryError::InvalidResponse)?;

            Ok(FetchCrateVersionDependenciesRepositoryOutput {
                dependencies: parsed
                    .dependencies
                    .into_iter()
                    .map(|d| RepositoryDependency {
                        name: d.crate_id,
                        req: d.req,
                        kind: match d.kind.as_str() {
                            "dev" => RepositoryDependencyKind::Dev,
                            "build" => RepositoryDependencyKind::Build,
                            // crates.io uses "normal" but be permissive
                            // against unknown future kinds — default to
                            // Normal rather than dropping the dep.
                            _ => RepositoryDependencyKind::Normal,
                        },
                        optional: d.optional,
                    })
                    .collect(),
            })
        })
    }
}

/// Consolidated status check: 404 → `NotFound`, other non-2xx →
/// `UpstreamStatus` with body, 2xx → return body bytes. Centralised so
/// the three endpoint impls share identical error routing.
///
/// Reads the body in streaming chunks and aborts with
/// [`CratesIoRepositoryError::PayloadTooLarge`] once `limit_bytes` is
/// exceeded — so a misbehaving mirror cannot exhaust memory with an
/// inflated `Content-Length`. Error bodies are bounded by a smaller
/// fixed cap because they're only used as diagnostic text.
async fn check_status_and_read_body(
    response: reqwest::Response,
    url: &str,
    limit_bytes: usize,
) -> Result<Vec<u8>, CratesIoRepositoryError> {
    const ERROR_BODY_CAP: usize = 16 * 1024;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(CratesIoRepositoryError::NotFound {
            url: url.to_string(),
        });
    }
    if !status.is_success() {
        let body_bytes = read_body_bounded(response, url, ERROR_BODY_CAP)
            .await
            .unwrap_or_default();
        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        return Err(CratesIoRepositoryError::UpstreamStatus {
            status,
            url: url.to_string(),
            body,
        });
    }
    read_body_bounded(response, url, limit_bytes).await
}

/// Stream a response body into memory, aborting once the running size
/// exceeds `limit_bytes`. Uses [`reqwest::Response::chunk`] rather
/// than `bytes()` so a 10 GB `Content-Length` cannot allocate before
/// the cap fires.
async fn read_body_bounded(
    mut response: reqwest::Response,
    url: &str,
    limit_bytes: usize,
) -> Result<Vec<u8>, CratesIoRepositoryError> {
    let mut acc: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if acc.len().saturating_add(chunk.len()) > limit_bytes {
            return Err(CratesIoRepositoryError::PayloadTooLarge {
                url: url.to_string(),
                limit_bytes,
            });
        }
        acc.extend_from_slice(&chunk);
    }
    Ok(acc)
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

#[derive(Debug, Deserialize)]
struct CratesIoCrateResponse {
    #[serde(rename = "crate")]
    krate: CratesIoRawCrateAggregate,
    versions: Vec<CratesIoRawCrateVersion>,
}

#[derive(Debug, Deserialize)]
struct CratesIoRawCrateAggregate {
    name: String,
    max_version: String,
    #[serde(default)]
    max_stable_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CratesIoRawCrateVersion {
    num: String,
    yanked: bool,
    created_at: String,
    #[serde(default)]
    features: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct CratesIoDependenciesResponse {
    dependencies: Vec<CratesIoRawDependency>,
}

#[derive(Debug, Deserialize)]
struct CratesIoRawDependency {
    /// crates.io's API field name; despite "id" this is the
    /// dependency's crate name as a string.
    crate_id: String,
    req: String,
    kind: String,
    optional: bool,
}

/// In-memory stub used by unit tests across the crate. Gated on `cfg(test)`
/// so it never ships in release builds and is invisible to integration
/// tests in `tests/` — those should exercise the real repository through
/// a `wiremock`-backed HTTP upstream.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CratesIoRepositoryStub {
    queue: tokio::sync::Mutex<Vec<SearchCratesResult>>,
    crate_queue: tokio::sync::Mutex<Vec<FetchCrateResult>>,
    deps_queue: tokio::sync::Mutex<Vec<FetchCrateVersionDependenciesResult>>,
    /// Captured `crate_name` strings for every per-crate call
    /// (`fetch_crate` and `fetch_crate_version_dependencies`), in
    /// arrival order. Lets unit tests pin name-normalisation policy
    /// (e.g. that the use case lowercases before talking upstream).
    seen_crate_names: tokio::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl CratesIoRepositoryStub {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn enqueue(&self, result: SearchCratesResult) {
        self.queue.lock().await.push(result);
    }

    pub(crate) async fn enqueue_crate(&self, result: FetchCrateResult) {
        self.crate_queue.lock().await.push(result);
    }

    pub(crate) async fn enqueue_dependencies(&self, result: FetchCrateVersionDependenciesResult) {
        self.deps_queue.lock().await.push(result);
    }

    /// Snapshot of every `crate_name` the stub has been asked about,
    /// across both per-crate trait methods, in arrival order.
    pub(crate) async fn seen_crate_names(&self) -> Vec<String> {
        self.seen_crate_names.lock().await.clone()
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

    fn fetch_crate(&self, input: FetchCrateInput) -> BoxFuture<'_, FetchCrateResult> {
        Box::pin(async move {
            self.seen_crate_names
                .lock()
                .await
                .push(input.crate_name.clone());
            self.crate_queue.lock().await.pop().unwrap_or_else(|| {
                Err(CratesIoRepositoryError::NotFound {
                    url: format!("stub://{}", input.crate_name),
                })
            })
        })
    }

    fn fetch_crate_version_dependencies(
        &self,
        input: FetchCrateVersionDependenciesInput,
    ) -> BoxFuture<'_, FetchCrateVersionDependenciesResult> {
        Box::pin(async move {
            self.seen_crate_names
                .lock()
                .await
                .push(input.crate_name.clone());
            self.deps_queue.lock().await.pop().unwrap_or_else(|| {
                Err(CratesIoRepositoryError::NotFound {
                    url: format!("stub://{}/{}", input.crate_name, input.version),
                })
            })
        })
    }
}
