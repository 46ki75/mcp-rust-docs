use std::sync::Arc;

use crate::Server;
use crate::crates_io::repository::{CratesIoRepository, CratesIoRepositoryImpl};
use crate::crates_io::use_case::CratesIoUseCase;
use crate::error::Error;

/// Default upstream base URL — the public crates.io registry.
pub const CRATES_IO_BASE_URL: &str = "https://crates.io";

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
    base_url: String,
    user_agent: String,
    http: Option<reqwest::Client>,
    repository: Option<Arc<dyn CratesIoRepository>>,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            base_url: CRATES_IO_BASE_URL.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            http: None,
            repository: None,
        }
    }
}

impl ServerBuilder {
    /// Override the registry base URL. Defaults to
    /// [`CRATES_IO_BASE_URL`]. Pass a wiremock URL in tests, or a
    /// registry mirror in production.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
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
    /// connection pool across many tools, or applying custom timeouts.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    /// Inject a fully-formed repository implementation. When set,
    /// short-circuits the HTTP client setup entirely. Mostly useful
    /// for advanced wiring; tests usually use the `cfg(test)` stub.
    pub fn crates_io_repository(mut self, repository: Arc<dyn CratesIoRepository>) -> Self {
        self.repository = Some(repository);
        self
    }

    /// Finalize the builder. Constructs the HTTP client (if one
    /// wasn't supplied) and wires up the repository, use case, and
    /// server in that order.
    pub fn build(self) -> Result<Server, Error> {
        let repository: Arc<dyn CratesIoRepository> = match self.repository {
            Some(repository) => repository,
            None => {
                let http = match self.http {
                    Some(client) => client,
                    None => reqwest::Client::builder()
                        .user_agent(self.user_agent)
                        .build()?,
                };
                Arc::new(CratesIoRepositoryImpl::new(http, self.base_url))
            }
        };

        let use_case = Arc::new(CratesIoUseCase::new(repository));
        Ok(Server::with_use_case(use_case))
    }
}
