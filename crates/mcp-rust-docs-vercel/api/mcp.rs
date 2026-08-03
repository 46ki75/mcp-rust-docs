//! Vercel entry point for the MCP server.
//!
//! Serves the same streamable-HTTP transport as `mcp-rust-docs http`,
//! adapted for serverless execution:
//!
//! - **Stateless mode.** Consecutive requests from one client can land
//!   on different Fluid-compute instances, so `Mcp-Session-Id` sessions
//!   held in one instance's memory would 404 on the next. All four
//!   tools are pure request/response (no sampling, elicitation, or
//!   subscriptions), so nothing is lost by disabling sessions.
//! - **Plain JSON responses** instead of SSE framing — allowed by the
//!   Streamable HTTP spec (2025-06-18) and cheaper for single-shot
//!   tool calls.
//! - **Host allowlist from Vercel env vars.** rmcp's DNS-rebinding
//!   guard defaults to loopback-only, which would reject every request
//!   arriving via the deployment URL. See [`allowed_hosts`].

use mcp_rust_docs::{CRATES_IO_BASE_URL, DOCS_RS_BASE_URL, Server};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use tower::ServiceBuilder;
use tracing_subscriber::EnvFilter;
use vercel_runtime::axum::VercelLayer;

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let env = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());

    let crates_io_base_url =
        env("MCP_CRATES_IO_BASE_URL").unwrap_or_else(|| CRATES_IO_BASE_URL.to_string());
    let docs_rs_base_url =
        env("MCP_DOCS_RS_BASE_URL").unwrap_or_else(|| DOCS_RS_BASE_URL.to_string());
    let docs_rs_cache = env("MCP_DOCS_RS_CACHE").is_none_or(|value| value != "false");
    let allowed_hosts = allowed_hosts(&env);

    tracing::info!(
        %crates_io_base_url,
        %docs_rs_base_url,
        %docs_rs_cache,
        ?allowed_hosts,
        "starting mcp-rust-docs on Vercel",
    );

    // Build the Server (and its `reqwest::Client`) ONCE and clone it in
    // the factory, same as `run_http` in the main binary — cloning is
    // cheap (use cases are behind `Arc`s) and reuses the connection
    // pool across requests handled by this instance.
    let server_template = Server::builder()
        .crates_io_base_url(crates_io_base_url)
        .docs_rs_base_url(docs_rs_base_url)
        .docs_rs_cache_enabled(docs_rs_cache)
        .build()?;

    let service = StreamableHttpService::new(
        move || Ok(server_template.clone()),
        NeverSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_allowed_hosts(allowed_hosts),
    );

    // Rewrites preserve the original request path, so the router sees
    // `/mcp`, not `/api/mcp` — mount at the public path.
    let router = axum::Router::new().nest_service("/mcp", service);

    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(router);
    vercel_runtime::run(app).await
}

/// Assemble the `Host`-header allowlist for rmcp's DNS-rebinding guard.
///
/// Vercel publishes its own hostnames through system env vars
/// (`VERCEL_URL` for this deployment, `VERCEL_BRANCH_URL` /
/// `VERCEL_PROJECT_PRODUCTION_URL` for the stable aliases), so those
/// are folded in automatically. Custom domains are not exposed that
/// way — list them in the comma-separated `MCP_ALLOWED_HOSTS`.
/// Loopback hosts stay allowed so `vercel dev` keeps working.
fn allowed_hosts(env: &impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];

    for key in [
        "VERCEL_URL",
        "VERCEL_BRANCH_URL",
        "VERCEL_PROJECT_PRODUCTION_URL",
    ] {
        hosts.extend(env(key));
    }

    if let Some(extra) = env("MCP_ALLOWED_HOSTS") {
        hosts.extend(
            extra
                .split(',')
                .map(str::trim)
                .filter(|host| !host.is_empty())
                .map(String::from),
        );
    }

    hosts.sort();
    hosts.dedup();
    hosts
}

#[cfg(test)]
mod tests {
    use super::allowed_hosts;

    fn env_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn defaults_to_loopback_only_without_vercel_env() {
        let hosts = allowed_hosts(&env_from(&[]));
        assert_eq!(hosts, ["127.0.0.1", "::1", "localhost"]);
    }

    #[test]
    fn folds_in_vercel_system_urls() {
        let hosts = allowed_hosts(&env_from(&[
            ("VERCEL_URL", "app-abc123.vercel.app"),
            ("VERCEL_PROJECT_PRODUCTION_URL", "app.vercel.app"),
        ]));
        assert!(hosts.contains(&"app-abc123.vercel.app".to_string()));
        assert!(hosts.contains(&"app.vercel.app".to_string()));
    }

    #[test]
    fn splits_and_trims_mcp_allowed_hosts() {
        let hosts = allowed_hosts(&env_from(&[(
            "MCP_ALLOWED_HOSTS",
            "docs.example.com , api.example.com,,",
        )]));
        assert!(hosts.contains(&"docs.example.com".to_string()));
        assert!(hosts.contains(&"api.example.com".to_string()));
        assert!(!hosts.contains(&"".to_string()));
    }

    #[test]
    fn deduplicates_overlapping_sources() {
        let hosts = allowed_hosts(&env_from(&[
            ("VERCEL_URL", "app.vercel.app"),
            ("MCP_ALLOWED_HOSTS", "app.vercel.app"),
        ]));
        assert_eq!(
            hosts
                .iter()
                .filter(|host| *host == "app.vercel.app")
                .count(),
            1
        );
    }
}
