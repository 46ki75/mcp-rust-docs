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
use crate::docs_rs::schema::DocsRsCrate;

/// Format versions this build knows how to deserialize, in dispatch
/// preference order (highest first). Each entry corresponds to a
/// cargo-renamed `rustdoc-types*` dep and a matching arm in
/// [`parse_dispatch`].
pub(crate) const SUPPORTED_FORMAT_VERSIONS: &[u32] = &[
    rustdoc_types::FORMAT_VERSION,
    rustdoc_types_56::FORMAT_VERSION,
];

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
    /// `/crate/{name}/{version}/json/{format}.zst`, decompress it
    /// (bounded by [`MAX_DECOMPRESSED_BYTES`]), and deserialize into
    /// the normalized [`DocsRsCrate`][crate::docs_rs::schema::DocsRsCrate].
    ///
    /// The repository inspects the JSON's `format_version` and
    /// dispatches to whichever `rustdoc-types` crate models that
    /// schema; unknown versions produce
    /// [`DocsRsRepositoryError::FormatVersionUnsupported`]. The
    /// decompressed-and-parsed crate is wrapped in `Arc` so the use
    /// case can pass it around without cloning a multi-MB structure.
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
            let parsed: Result<Arc<DocsRsCrate>, DocsRsRepositoryError> =
                tokio::task::spawn_blocking(move || {
                    let decompressed = decompress_zstd_bounded(&compressed, &final_url_owned)?;
                    parse_dispatch(&decompressed, &final_url_owned).map(Arc::new)
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

/// Peek at the JSON's `format_version` and dispatch to the matching
/// upstream deserializer, normalizing the result into [`DocsRsCrate`].
///
/// The probe-parse only reads the top-level `format_version` field —
/// serde tokenizes the rest of the document but builds no allocations
/// for the schema, so the cost is bounded. We pay it once so the full
/// parse can run against the correct schema and produce actionable
/// errors instead of cryptic mid-stream `missing field` failures when
/// docs.rs serves a format we don't model.
fn parse_dispatch(bytes: &[u8], url: &str) -> Result<DocsRsCrate, DocsRsRepositoryError> {
    #[derive(serde::Deserialize)]
    struct FormatVersionProbe {
        format_version: u32,
    }

    let probe = serde_json::from_slice::<FormatVersionProbe>(bytes).map_err(|source| {
        DocsRsRepositoryError::InvalidRustdocJson {
            url: url.to_string(),
            source,
        }
    })?;

    match probe.format_version {
        v if v == rustdoc_types::FORMAT_VERSION => {
            serde_json::from_slice::<rustdoc_types::Crate>(bytes)
                .map(DocsRsCrate::from)
                .map_err(|source| DocsRsRepositoryError::InvalidRustdocJson {
                    url: url.to_string(),
                    source,
                })
        }
        v if v == rustdoc_types_56::FORMAT_VERSION => {
            serde_json::from_slice::<rustdoc_types_56::Crate>(bytes)
                .map(DocsRsCrate::from)
                .map_err(|source| DocsRsRepositoryError::InvalidRustdocJson {
                    url: url.to_string(),
                    source,
                })
        }
        actual => Err(DocsRsRepositoryError::FormatVersionUnsupported {
            url: url.to_string(),
            actual,
            supported: SUPPORTED_FORMAT_VERSIONS.to_vec(),
        }),
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

    /// Snapshot of every URL the stub has been asked to fetch, in the
    /// order requests arrived. Useful for tests that need to assert
    /// the fallback chain walked the expected versions.
    pub(crate) async fn seen_urls(&self) -> Vec<String> {
        self.seen.lock().await.clone()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    const ANYHOW_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/anyhow_rustdoc.json.zst");
    const SERDE_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/serde_rustdoc.json.zst");

    fn decompress(zst: &[u8]) -> Vec<u8> {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(zst).expect("zstd header");
        let mut out = Vec::with_capacity(4 * 1024 * 1024);
        decoder.read_to_end(&mut out).expect("zstd body");
        out
    }

    fn anyhow_bytes() -> &'static [u8] {
        static CACHED: OnceLock<Vec<u8>> = OnceLock::new();
        CACHED.get_or_init(|| decompress(ANYHOW_FIXTURE))
    }

    fn serde_bytes() -> &'static [u8] {
        static CACHED: OnceLock<Vec<u8>> = OnceLock::new();
        CACHED.get_or_init(|| decompress(SERDE_FIXTURE))
    }

    #[test]
    fn parse_dispatch_routes_format_57_via_current_rustdoc_types() {
        let out = parse_dispatch(anyhow_bytes(), "test://anyhow")
            .expect("anyhow fixture is format 57; dispatch must succeed");
        // Sanity: real anyhow has hundreds of indexed items. We're
        // confirming the From conversion populated the normalized
        // shape, not just that deserialization succeeded.
        assert!(!out.index.is_empty(), "index empty after dispatch");
        assert!(!out.paths.is_empty(), "paths empty after dispatch");
    }

    #[test]
    fn parse_dispatch_routes_format_56_via_renamed_dep() {
        let out = parse_dispatch(serde_bytes(), "test://serde")
            .expect("serde fixture is format 56; dispatch must succeed via rustdoc-types-56");
        assert!(!out.index.is_empty(), "index empty after dispatch");
        assert!(!out.paths.is_empty(), "paths empty after dispatch");
    }

    #[test]
    fn parse_dispatch_reports_unsupported_format() {
        let payload = br#"{"format_version":9999,"junk":true}"#;
        let err = parse_dispatch(payload, "test://made-up")
            .expect_err("format 9999 isn't supported; must error");
        match err {
            DocsRsRepositoryError::FormatVersionUnsupported {
                actual, supported, ..
            } => {
                assert_eq!(actual, 9999);
                assert_eq!(supported, SUPPORTED_FORMAT_VERSIONS.to_vec());
            }
            other => panic!("expected FormatVersionUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn parse_dispatch_reports_invalid_when_probe_cant_read_format_version() {
        // Truly malformed payload (no `format_version` field at all)
        // surfaces through InvalidRustdocJson, distinguishing
        // "schema we don't know" from "bytes aren't even rustdoc JSON".
        let payload = br#"{"hello":"world"}"#;
        let err = parse_dispatch(payload, "test://garbage")
            .expect_err("payload without format_version must error");
        assert!(
            matches!(err, DocsRsRepositoryError::InvalidRustdocJson { .. }),
            "expected InvalidRustdocJson, got {err:?}",
        );
    }
}
