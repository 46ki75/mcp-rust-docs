use std::sync::Arc;
use std::time::Duration;

use crate::Server;
use crate::crates_io::repository::{CratesIoRepository, CratesIoRepositoryImpl};
use crate::crates_io::use_case::CratesIoUseCase;
use crate::docs_rs::repository::{CachingDocsRsRepository, DocsRsRepository, DocsRsRepositoryImpl};
use crate::docs_rs::use_case::DocsRsUseCase;
use crate::error::Error;

/// Default crates.io upstream — the public registry.
pub const CRATES_IO_BASE_URL: &str = "https://crates.io";

/// Default docs.rs upstream — the public documentation host.
pub const DOCS_RS_BASE_URL: &str = "https://docs.rs";

/// Default overall HTTP request timeout applied to the built-in
/// `reqwest::Client`.
///
/// Without this cap a stuck upstream socket pins the request handler
/// indefinitely — the streamable-HTTP transport has no per-call
/// deadline of its own. 30s is generous for docs.rs's largest rustdoc
/// JSON downloads while still surfacing a real outage as an error
/// instead of a hang.
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Default TCP-connect timeout applied to the built-in
/// `reqwest::Client`. Distinct from the overall request timeout so a
/// dead upstream fails fast at the SYN-ACK stage rather than burning
/// the full request budget.
pub const DEFAULT_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default per-response body size cap applied to both upstream
/// repositories.
///
/// The largest payload either upstream serves in practice is the
/// zstd-compressed rustdoc JSON for huge crates — tens of MB. 128 MB
/// leaves headroom for outliers while keeping a misbehaving mirror
/// from pinning the process with a 10 GB body. Decompressed JSON is
/// independently capped at 64 MB inside the docs.rs repository.
pub const DEFAULT_UPSTREAM_BODY_BYTES: usize = 128 * 1024 * 1024;

/// Default `User-Agent` header sent by the built-in HTTP client.
///
/// crates.io requires a contactable User-Agent on every request; this
/// embeds the crate version and source URL so the registry can reach
/// out if anything misbehaves.
pub const DEFAULT_USER_AGENT: &str = concat!(
    "mcp-rust-docs/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/46ki75/mcp-rust-docs)",
);

/// Fluent builder for [`Server`].
///
/// All fields are optional; calling [`build`](Self::build) with no
/// overrides reproduces [`Server::new`][crate::Server::new]. Useful
/// when tests want to point at a wiremock URL or when ops needs to
/// inject a pre-configured `reqwest::Client`.
pub struct ServerBuilder {
    crates_io_base_url: String,
    docs_rs_base_url: String,
    user_agent: String,
    http: Option<reqwest::Client>,
    http_timeout: Duration,
    http_connect_timeout: Duration,
    upstream_body_size_limit: usize,
    crates_io_repository: Option<Arc<dyn CratesIoRepository>>,
    docs_rs_repository: Option<Arc<dyn DocsRsRepository>>,
    docs_rs_cache_enabled: bool,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            crates_io_base_url: CRATES_IO_BASE_URL.to_string(),
            docs_rs_base_url: DOCS_RS_BASE_URL.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            http: None,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            http_connect_timeout: DEFAULT_HTTP_CONNECT_TIMEOUT,
            upstream_body_size_limit: DEFAULT_UPSTREAM_BODY_BYTES,
            crates_io_repository: None,
            docs_rs_repository: None,
            docs_rs_cache_enabled: true,
        }
    }
}

impl ServerBuilder {
    /// Override the crates.io registry base URL. Defaults to
    /// [`CRATES_IO_BASE_URL`]. Pass a wiremock URL in tests, or a
    /// registry mirror in production.
    pub fn crates_io_base_url(mut self, url: impl Into<String>) -> Self {
        self.crates_io_base_url = url.into();
        self
    }

    /// Override the docs.rs base URL. Defaults to
    /// [`DOCS_RS_BASE_URL`]. Pass a wiremock URL in tests or a mirror
    /// like `docs.rs.local` in production.
    pub fn docs_rs_base_url(mut self, url: impl Into<String>) -> Self {
        self.docs_rs_base_url = url.into();
        self
    }

    /// Override the `User-Agent` header sent by the built-in HTTP
    /// client. Ignored when [`http_client`](Self::http_client) is
    /// also set (the supplied client owns its own headers).
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Supply a pre-built `reqwest::Client`. Useful for sharing one
    /// connection pool across both tools, or applying custom timeouts.
    ///
    /// **Caveat:** when a client is supplied here, the
    /// [`http_timeout`](Self::http_timeout) and
    /// [`http_connect_timeout`](Self::http_connect_timeout) settings
    /// are ignored — the injected client owns its own timeout
    /// configuration.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    /// Override the overall HTTP request timeout applied to the
    /// built-in client. Defaults to [`DEFAULT_HTTP_TIMEOUT`].
    /// No-op when [`http_client`](Self::http_client) is also set.
    pub fn http_timeout(mut self, timeout: Duration) -> Self {
        self.http_timeout = timeout;
        self
    }

    /// Override the TCP-connect timeout applied to the built-in
    /// client. Defaults to [`DEFAULT_HTTP_CONNECT_TIMEOUT`].
    /// No-op when [`http_client`](Self::http_client) is also set.
    pub fn http_connect_timeout(mut self, timeout: Duration) -> Self {
        self.http_connect_timeout = timeout;
        self
    }

    /// Override the per-response upstream body-size cap applied by
    /// both repositories. Defaults to [`DEFAULT_UPSTREAM_BODY_BYTES`].
    ///
    /// Ignored when [`crates_io_repository`](Self::crates_io_repository)
    /// or [`docs_rs_repository`](Self::docs_rs_repository) is supplied
    /// for the corresponding side — the injected repository owns its
    /// own body limit.
    pub fn upstream_body_size_limit(mut self, limit: usize) -> Self {
        self.upstream_body_size_limit = limit;
        self
    }

    /// Inject a fully-formed crates.io repository. When set,
    /// short-circuits HTTP client setup for crates.io. Mostly useful
    /// for advanced wiring; tests usually use the `cfg(test)` stub.
    pub fn crates_io_repository(mut self, repository: Arc<dyn CratesIoRepository>) -> Self {
        self.crates_io_repository = Some(repository);
        self
    }

    /// Inject a fully-formed docs.rs repository. Same caveats as
    /// [`crates_io_repository`](Self::crates_io_repository).
    ///
    /// **Caveat:** the injected repository is silently wrapped by the
    /// rustdoc-JSON cache when [`docs_rs_cache_enabled`](Self::docs_rs_cache_enabled)
    /// is `true` (the default). Tests that drive a stub or wiremock and
    /// assert exact upstream call counts (e.g. `.expect(N)`) MUST pass
    /// `false` to `docs_rs_cache_enabled` — otherwise repeat fetches
    /// hit the cache instead of the stub and the assertions undercount.
    pub fn docs_rs_repository(mut self, repository: Arc<dyn DocsRsRepository>) -> Self {
        self.docs_rs_repository = Some(repository);
        self
    }

    /// Toggle the in-process rustdoc-JSON cache. Default: on. Tests
    /// that assert exact upstream invocation counts against wiremock
    /// should disable it so a cache hit doesn't silently swallow a
    /// would-be second request. See
    /// [`CachingDocsRsRepository`][crate::docs_rs::repository::CachingDocsRsRepository]
    /// for what the cache covers and why HTML is not cached.
    pub fn docs_rs_cache_enabled(mut self, enabled: bool) -> Self {
        self.docs_rs_cache_enabled = enabled;
        self
    }

    /// Finalize the builder. Constructs the HTTP client (only when
    /// at least one repository wasn't supplied), wires up both
    /// repositories, both use cases, and the server in that order.
    pub fn build(self) -> Result<Server, Error> {
        // Lazily produce a shared `reqwest::Client` so callers that
        // injected both repositories don't pay for an unused DNS
        // resolver / connection pool. The closure captures `self.http`
        // and `self.user_agent` by `Option::take` semantics.
        let mut http_override = self.http;
        let user_agent = self.user_agent;
        let http_timeout = self.http_timeout;
        let http_connect_timeout = self.http_connect_timeout;
        let mut shared_http: Option<reqwest::Client> = None;
        let mut get_http = || -> Result<reqwest::Client, Error> {
            if let Some(client) = &shared_http {
                return Ok(client.clone());
            }
            let client = match http_override.take() {
                Some(c) => c,
                None => reqwest::Client::builder()
                    .user_agent(user_agent.clone())
                    .timeout(http_timeout)
                    .connect_timeout(http_connect_timeout)
                    .build()?,
            };
            shared_http = Some(client.clone());
            Ok(client)
        };

        let body_limit = self.upstream_body_size_limit;
        let crates_io_repository: Arc<dyn CratesIoRepository> = match self.crates_io_repository {
            Some(repository) => repository,
            None => Arc::new(
                CratesIoRepositoryImpl::new(get_http()?, self.crates_io_base_url)
                    .with_max_body_bytes(body_limit),
            ),
        };

        let docs_rs_repository: Arc<dyn DocsRsRepository> = match self.docs_rs_repository {
            Some(repository) => repository,
            None => {
                Arc::new(DocsRsRepositoryImpl::new(get_http()?).with_max_body_bytes(body_limit))
            }
        };
        let docs_rs_repository: Arc<dyn DocsRsRepository> = if self.docs_rs_cache_enabled {
            Arc::new(CachingDocsRsRepository::new(docs_rs_repository))
        } else {
            docs_rs_repository
        };

        let crates_io_use_case = Arc::new(CratesIoUseCase::new(crates_io_repository));
        let docs_rs_use_case = Arc::new(DocsRsUseCase::new(
            docs_rs_repository,
            self.docs_rs_base_url,
        ));
        Ok(Server::with_use_cases(crates_io_use_case, docs_rs_use_case))
    }
}
