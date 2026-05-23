/// In-process bounded TTL cache wrapping another [`DocsRsRepository`].
pub mod cache;
/// Per-error-variant docs for repository failures.
pub mod error;
/// Input types accepted by the repository.
pub mod input;
/// Output types returned by the repository.
pub mod output;

use std::io::Read;
use std::sync::Arc;

pub use self::cache::CachingDocsRsRepository;
pub use self::error::DocsRsRepositoryError;
pub use self::input::{FetchCrateDocsRepositoryInput, FetchRustdocJsonRepositoryInput};
pub use self::output::{FetchCrateDocsRepositoryOutput, FetchRustdocJsonRepositoryOutput};

use crate::crates_io::repository::BoxFuture;

/// Convenience alias for the HTML-fetch result shape.
pub type FetchCrateDocsResult = Result<FetchCrateDocsRepositoryOutput, DocsRsRepositoryError>;

/// Convenience alias for the rustdoc-JSON-fetch result shape.
pub type FetchRustdocJsonResult = Result<FetchRustdocJsonRepositoryOutput, DocsRsRepositoryError>;

/// Cap on the decompressed rustdoc-JSON payload size.
///
/// docs.rs serves a zstd-compressed JSON file that decompresses to
/// hundreds of KB for small crates and tens of MB for the largest
/// (e.g. `bevy`, `tokio`). The cap exists so a single huge crate
/// (or a malicious upstream) can't exhaust memory. Set high enough
/// that real-world crates work; low enough that the worst case is
/// bounded.
const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

/// Repository abstraction over docs.rs HTTP fetches.
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

    /// Fetch the zstd-compressed rustdoc JSON for a crate from
    /// `/crate/{name}/{version}/json.zst`, decompress it (bounded by
    /// [`MAX_DECOMPRESSED_BYTES`]), and deserialize into
    /// [`rustdoc_types::Crate`]. The decompressed-and-parsed crate
    /// is wrapped in `Arc` so the use case can pass it around without
    /// cloning a multi-MB structure.
    fn fetch_rustdoc_json(
        &self,
        input: FetchRustdocJsonRepositoryInput,
    ) -> BoxFuture<'_, FetchRustdocJsonResult>;
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

    fn fetch_rustdoc_json(
        &self,
        input: FetchRustdocJsonRepositoryInput,
    ) -> BoxFuture<'_, FetchRustdocJsonResult> {
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

            let compressed = response.bytes().await?;

            // Streaming-decompress and parse on the thread pool — both
            // are CPU-bound and would otherwise hog the runtime worker.
            // Move the decoded URL into the closure to keep error
            // payloads accurate.
            let final_url_owned = final_url.clone();
            let parsed: Result<Arc<rustdoc_types::Crate>, DocsRsRepositoryError> =
                tokio::task::spawn_blocking(move || {
                    let decompressed = decompress_zstd_bounded(&compressed, &final_url_owned)?;
                    let crate_json = serde_json::from_slice::<rustdoc_types::Crate>(&decompressed)
                        .map_err(|source| DocsRsRepositoryError::InvalidRustdocJson {
                            url: final_url_owned.clone(),
                            source,
                        })?;
                    if crate_json.format_version != rustdoc_types::FORMAT_VERSION {
                        return Err(DocsRsRepositoryError::FormatVersionMismatch {
                            url: final_url_owned,
                            actual: crate_json.format_version,
                            expected: rustdoc_types::FORMAT_VERSION,
                        });
                    }
                    Ok(Arc::new(crate_json))
                })
                .await
                .map_err(|join_err| DocsRsRepositoryError::Decompression {
                    url: final_url.clone(),
                    source: std::io::Error::other(join_err.to_string()),
                })?;
            let crate_json = parsed?;

            Ok(FetchRustdocJsonRepositoryOutput {
                final_url,
                crate_json,
            })
        })
    }
}

/// Streaming-decompress a zstd payload while enforcing a cap on the
/// output size. Reads `MAX_DECOMPRESSED_BYTES + 1` bytes at most;
/// if the extra byte is consumed, the cap fired and we report
/// [`DocsRsRepositoryError::PayloadTooLarge`].
fn decompress_zstd_bounded(compressed: &[u8], url: &str) -> Result<Vec<u8>, DocsRsRepositoryError> {
    let decoder = ruzstd::decoding::StreamingDecoder::new(compressed).map_err(|err| {
        DocsRsRepositoryError::Decompression {
            url: url.to_string(),
            source: std::io::Error::other(err.to_string()),
        }
    })?;
    // `take(MAX + 1)` so a payload that exactly equals the cap is
    // accepted, but anything larger trips the check below.
    let cap = MAX_DECOMPRESSED_BYTES;
    let mut limited = decoder.take((cap as u64).saturating_add(1));
    let mut out = Vec::with_capacity(64 * 1024);
    limited
        .read_to_end(&mut out)
        .map_err(|source| DocsRsRepositoryError::Decompression {
            url: url.to_string(),
            source,
        })?;
    if out.len() > cap {
        return Err(DocsRsRepositoryError::PayloadTooLarge {
            url: url.to_string(),
            limit_bytes: cap,
        });
    }
    Ok(out)
}

/// In-memory stub used by unit tests across the crate. Gated on
/// `cfg(test)` so it never ships in release builds and is invisible
/// to integration tests in `tests/`.
///
/// Uses `Mutex<Vec<...>>` (not the `Fn` pattern from `CountingRepo`
/// in `cache.rs`) because its responses are queue-based — `pop()` is
/// an `FnMut` operation, and the mutex is what makes that sound under
/// the shared `&self` repository trait. The two stubs serve different
/// test shapes: this one consumes pre-enqueued results, `CountingRepo`
/// invokes a single closure repeatedly. Don't "harmonize" them.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct DocsRsRepositoryStub {
    queue: tokio::sync::Mutex<Vec<FetchCrateDocsResult>>,
    json_queue: tokio::sync::Mutex<Vec<FetchRustdocJsonResult>>,
    /// Captured URLs so unit tests can assert the use case built the
    /// right docs.rs URL. Both HTML and JSON fetches share this log
    /// because the URL alone is enough to disambiguate.
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

    pub(crate) async fn enqueue_json(&self, result: FetchRustdocJsonResult) {
        self.json_queue.lock().await.push(result);
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

    fn fetch_rustdoc_json(
        &self,
        input: FetchRustdocJsonRepositoryInput,
    ) -> BoxFuture<'_, FetchRustdocJsonResult> {
        Box::pin(async move {
            self.seen.lock().await.push(input.url.clone());
            // Tests that don't pre-enqueue a JSON result get a NotFound
            // — silently returning an empty `Crate` would mask missing
            // setup, since `rustdoc_types::Crate` has no `Default`.
            self.json_queue
                .lock()
                .await
                .pop()
                .unwrap_or_else(|| Err(DocsRsRepositoryError::NotFound { url: input.url }))
        })
    }
}
