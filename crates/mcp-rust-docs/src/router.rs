use std::sync::Arc;

use crate::Server;
use crate::crates_io::repository::{CratesIoRepository, CratesIoRepositoryImpl};
use crate::crates_io::use_case::CratesIoUseCase;
use crate::error::Error;

pub const CRATES_IO_BASE_URL: &str = "https://crates.io";

pub const DEFAULT_USER_AGENT: &str = concat!(
    "mcp-rust-docs/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/46ki75/mcp-rust-docs)",
);

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
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    pub fn crates_io_repository(mut self, repository: Arc<dyn CratesIoRepository>) -> Self {
        self.repository = Some(repository);
        self
    }

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
