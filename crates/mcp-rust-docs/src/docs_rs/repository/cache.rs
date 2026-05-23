//! Caching decorator for [`DocsRsRepository`].
//!
//! Wraps another repository and short-circuits successful
//! `fetch_rustdoc_json` calls on subsequent requests for the same URL.
//! The HTML side (`fetch_crate_docs`) is pass-through — HTML hits a
//! different page per call in practice, so caching it would mostly
//! waste capacity. Revisit if `all.html` re-fetches show up in traces.
//!
//! ### What is cached
//!
//! Successful `FetchRustdocJsonRepositoryOutput` values, keyed by the
//! requested URL string. The URL already encodes
//! `(crate_name, version_or_latest)` so we don't need a structured key.
//! Errors are **not** cached: agents retry expecting fixes, and 404s
//! can be transient (newly-published crate, docs.rs build pipeline lag).
//!
//! ### Why no singleflight
//!
//! `moka::future::Cache::try_get_with` would deduplicate concurrent
//! identical fetches, but it returns `Result<V, Arc<E>>` on failure —
//! and `DocsRsRepositoryError` is not `Clone` (it wraps `reqwest::Error`
//! and `serde_json::Error`), so callers couldn't pattern-match the
//! cached `Arc<E>` against the existing variants the tool layer uses.
//! For an MCP server with a typical single-editor-session workload, the
//! concurrent-identical-miss case is rare; we accept a small amount of
//! duplicated work in exchange for keeping error types owned.
//!
//! ### Why the URL is the key
//!
//! `latest` and `1.2.3` resolve to different cache entries even when
//! they're the same release — that's a known small inefficiency, paid
//! for in simplicity. The TTL bounds staleness for the `latest` case
//! globally; for concrete versions it's harmless overcaching since
//! published versions are immutable.

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

use crate::crates_io::repository::BoxFuture;
use crate::docs_rs::repository::{
    DocsRsRepository, FetchCrateDocsRepositoryInput, FetchCrateDocsResult,
    FetchRustdocJsonRepositoryInput, FetchRustdocJsonRepositoryOutput, FetchRustdocJsonResult,
};

/// Default time-to-live for cached rustdoc JSON entries. Bounds
/// `latest` staleness globally; concrete versions are immutable so
/// this is just an "overcaching is harmless" upper bound for them.
pub const DEFAULT_RUSTDOC_CACHE_TTL: Duration = Duration::from_secs(600);

/// Default maximum number of distinct `(crate, version)` rustdoc JSON
/// payloads held in memory. A big crate's parsed `Crate` is a few MB,
/// so 16 entries caps worst-case memory at the low tens of MB.
pub const DEFAULT_RUSTDOC_CACHE_CAPACITY: u64 = 16;

/// Tunables for [`CachingDocsRsRepository`]. Defaults to
/// [`DEFAULT_RUSTDOC_CACHE_TTL`] / [`DEFAULT_RUSTDOC_CACHE_CAPACITY`].
#[derive(Debug, Clone, Copy)]
pub struct CachingDocsRsRepositoryConfig {
    /// How long a cached rustdoc JSON entry remains fresh.
    pub ttl: Duration,
    /// Maximum number of `(crate, version)` payloads to retain.
    pub max_entries: u64,
}

impl Default for CachingDocsRsRepositoryConfig {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_RUSTDOC_CACHE_TTL,
            max_entries: DEFAULT_RUSTDOC_CACHE_CAPACITY,
        }
    }
}

/// Decorator that adds an in-process bounded TTL cache (W-TinyLFU
/// eviction, via `moka`) to `fetch_rustdoc_json`. `fetch_crate_docs`
/// passes through unchanged.
pub struct CachingDocsRsRepository {
    inner: Arc<dyn DocsRsRepository>,
    rustdoc_cache: Cache<String, Arc<FetchRustdocJsonRepositoryOutput>>,
}

impl CachingDocsRsRepository {
    /// Wrap `inner` with a cache using
    /// [`CachingDocsRsRepositoryConfig::default`].
    pub fn new(inner: Arc<dyn DocsRsRepository>) -> Self {
        Self::with_config(inner, CachingDocsRsRepositoryConfig::default())
    }

    /// Wrap `inner` with a cache using the supplied configuration.
    pub fn with_config(
        inner: Arc<dyn DocsRsRepository>,
        config: CachingDocsRsRepositoryConfig,
    ) -> Self {
        let rustdoc_cache = Cache::builder()
            .time_to_live(config.ttl)
            .max_capacity(config.max_entries)
            .build();
        Self {
            inner,
            rustdoc_cache,
        }
    }
}

impl DocsRsRepository for CachingDocsRsRepository {
    fn fetch_crate_docs(
        &self,
        input: FetchCrateDocsRepositoryInput,
    ) -> BoxFuture<'_, FetchCrateDocsResult> {
        // Pass-through. See module docs for why HTML isn't cached.
        self.inner.fetch_crate_docs(input)
    }

    fn fetch_rustdoc_json(
        &self,
        input: FetchRustdocJsonRepositoryInput,
    ) -> BoxFuture<'_, FetchRustdocJsonResult> {
        Box::pin(async move {
            let key = input.url.clone();
            if let Some(cached) = self.rustdoc_cache.get(&key).await {
                return Ok((*cached).clone());
            }
            let output = self.inner.fetch_rustdoc_json(input).await?;
            self.rustdoc_cache
                .insert(key, Arc::new(output.clone()))
                .await;
            Ok(output)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::docs_rs::repository::DocsRsRepositoryError;

    type JsonResponder = Box<dyn FnMut(&str) -> FetchRustdocJsonResult + Send + 'static>;

    /// Counting repository: records how many times each method was
    /// invoked. Lets us assert "the inner repo was hit exactly N times"
    /// without juggling a separate stub queue per test.
    struct CountingRepo {
        json_calls: AtomicUsize,
        json_response: tokio::sync::Mutex<JsonResponder>,
    }

    impl CountingRepo {
        fn new<F>(f: F) -> Self
        where
            F: FnMut(&str) -> FetchRustdocJsonResult + Send + 'static,
        {
            Self {
                json_calls: AtomicUsize::new(0),
                json_response: tokio::sync::Mutex::new(Box::new(f)),
            }
        }

        fn json_call_count(&self) -> usize {
            self.json_calls.load(Ordering::SeqCst)
        }
    }

    impl DocsRsRepository for CountingRepo {
        fn fetch_crate_docs(
            &self,
            _input: FetchCrateDocsRepositoryInput,
        ) -> BoxFuture<'_, FetchCrateDocsResult> {
            unreachable!("HTML path is not exercised by cache tests")
        }

        fn fetch_rustdoc_json(
            &self,
            input: FetchRustdocJsonRepositoryInput,
        ) -> BoxFuture<'_, FetchRustdocJsonResult> {
            Box::pin(async move {
                self.json_calls.fetch_add(1, Ordering::SeqCst);
                let mut guard = self.json_response.lock().await;
                (guard)(&input.url)
            })
        }
    }

    fn ok_output(url: &str) -> FetchRustdocJsonResult {
        Ok(FetchRustdocJsonRepositoryOutput {
            final_url: url.to_string(),
            crate_json: stub_crate(),
        })
    }

    /// Decompress and parse the real anyhow fixture once per test
    /// process. The cache stores the `Arc<Crate>` opaquely — its
    /// contents aren't inspected here, but constructing a valid
    /// `rustdoc_types::Crate` by hand requires keeping up with every
    /// new field the upstream adds, so it's simpler to reuse the
    /// fixture the integration tests already maintain.
    fn stub_crate() -> Arc<rustdoc_types::Crate> {
        use std::io::Read;
        use std::sync::OnceLock;
        static CACHED: OnceLock<Arc<rustdoc_types::Crate>> = OnceLock::new();
        CACHED
            .get_or_init(|| {
                const FIXTURE: &[u8] =
                    include_bytes!("../../../tests/fixtures/anyhow_rustdoc.json.zst");
                let mut decoder =
                    ruzstd::decoding::StreamingDecoder::new(FIXTURE).expect("zstd decode");
                let mut decompressed = Vec::with_capacity(512 * 1024);
                decoder
                    .read_to_end(&mut decompressed)
                    .expect("zstd read_to_end");
                Arc::new(serde_json::from_slice(&decompressed).expect("parse anyhow fixture"))
            })
            .clone()
    }

    #[tokio::test]
    async fn second_call_for_same_url_hits_cache() {
        let inner = Arc::new(CountingRepo::new(ok_output));
        let cache = CachingDocsRsRepository::new(inner.clone());

        let url = "https://docs.rs/crate/anyhow/1.0.86/json.zst".to_string();
        let _ = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url: url.clone() })
            .await
            .expect("first fetch must succeed");
        let _ = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url: url.clone() })
            .await
            .expect("second fetch must succeed");

        assert_eq!(
            inner.json_call_count(),
            1,
            "second call should hit cache, not inner",
        );
    }

    #[tokio::test]
    async fn different_urls_are_cached_independently() {
        let inner = Arc::new(CountingRepo::new(ok_output));
        let cache = CachingDocsRsRepository::new(inner.clone());

        let a = "https://docs.rs/crate/anyhow/1.0.86/json.zst".to_string();
        let b = "https://docs.rs/crate/serde/1.0.200/json.zst".to_string();
        let _ = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url: a })
            .await;
        let _ = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url: b })
            .await;

        assert_eq!(inner.json_call_count(), 2);
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        // First call errors (NotFound); second call returns success.
        // The cache should NOT have stored the first call's failure,
        // so the second call must reach the inner and surface the OK.
        let counter = AtomicUsize::new(0);
        let inner = Arc::new(CountingRepo::new(move |url| {
            let call = counter.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Err(DocsRsRepositoryError::NotFound {
                    url: url.to_string(),
                })
            } else {
                ok_output(url)
            }
        }));
        let cache = CachingDocsRsRepository::new(inner.clone());

        let url = "https://docs.rs/crate/anyhow/1.0.86/json.zst".to_string();
        let first = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url: url.clone() })
            .await;
        assert!(matches!(first, Err(DocsRsRepositoryError::NotFound { .. })));

        let second = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url })
            .await;
        assert!(second.is_ok(), "second call must reach inner: {second:?}");
        assert_eq!(
            inner.json_call_count(),
            2,
            "both calls should reach inner since the error was not cached",
        );
    }

    #[tokio::test]
    async fn ttl_expiry_triggers_refetch() {
        let inner = Arc::new(CountingRepo::new(ok_output));
        let cache = CachingDocsRsRepository::with_config(
            inner.clone(),
            CachingDocsRsRepositoryConfig {
                ttl: Duration::from_millis(50),
                max_entries: 16,
            },
        );

        let url = "https://docs.rs/crate/anyhow/1.0.86/json.zst".to_string();
        let _ = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url: url.clone() })
            .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        // moka's TTL eviction is lazy; an explicit `run_pending_tasks`
        // forces the bookkeeping so the next `get` sees the expiry.
        cache.rustdoc_cache.run_pending_tasks().await;
        let _ = cache
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url })
            .await;

        assert_eq!(
            inner.json_call_count(),
            2,
            "post-TTL fetch should re-hit inner",
        );
    }
}
