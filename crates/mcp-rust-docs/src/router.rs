use std::sync::Arc;

use crate::Server;
use crate::crates_io::repository::{CratesIoRepository, CratesIoRepositoryImpl};
use crate::crates_io::use_case::CratesIoUseCase;
use crate::docs_rs::repository::{DocsRsRepository, DocsRsRepositoryImpl};
use crate::docs_rs::use_case::DocsRsUseCase;
use crate::error::Error;

/// Default crates.io upstream — the public registry.
pub const CRATES_IO_BASE_URL: &str = "https://crates.io";

/// Default docs.rs upstream — the public documentation host.
pub const DOCS_RS_BASE_URL: &str = "https://docs.rs";

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
    crates_io_repository: Option<Arc<dyn CratesIoRepository>>,
    docs_rs_repository: Option<Arc<dyn DocsRsRepository>>,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            crates_io_base_url: CRATES_IO_BASE_URL.to_string(),
            docs_rs_base_url: DOCS_RS_BASE_URL.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            http: None,
            crates_io_repository: None,
            docs_rs_repository: None,
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
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
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
    pub fn docs_rs_repository(mut self, repository: Arc<dyn DocsRsRepository>) -> Self {
        self.docs_rs_repository = Some(repository);
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
        let mut shared_http: Option<reqwest::Client> = None;
        let mut get_http = || -> Result<reqwest::Client, Error> {
            if let Some(client) = &shared_http {
                return Ok(client.clone());
            }
            let client = match http_override.take() {
                Some(c) => c,
                None => reqwest::Client::builder()
                    .user_agent(user_agent.clone())
                    .build()?,
            };
            shared_http = Some(client.clone());
            Ok(client)
        };

        let crates_io_repository: Arc<dyn CratesIoRepository> = match self.crates_io_repository {
            Some(repository) => repository,
            None => Arc::new(CratesIoRepositoryImpl::new(
                get_http()?,
                self.crates_io_base_url,
            )),
        };

        let docs_rs_repository: Arc<dyn DocsRsRepository> = match self.docs_rs_repository {
            Some(repository) => repository,
            None => Arc::new(DocsRsRepositoryImpl::new(get_http()?)),
        };

        let crates_io_use_case = Arc::new(CratesIoUseCase::new(crates_io_repository));
        let docs_rs_use_case = Arc::new(DocsRsUseCase::new(
            docs_rs_repository,
            self.docs_rs_base_url,
        ));
        Ok(Server::with_use_cases(crates_io_use_case, docs_rs_use_case))
    }
}
