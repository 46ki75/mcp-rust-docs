//! Unified entry point for the MCP server.
//!
//! Subcommands select the transport:
//! - `stdio` — line-buffered JSON-RPC over stdin/stdout
//! - `http`  — streamable HTTP, mounted at `/mcp`
//!
//! Shared options (registry URL, etc.) live at the top level and
//! accept env-var fallbacks so MCP hosts can inject them without
//! rewriting argv.

use clap::{Args, Parser, Subcommand};
use mcp_rust_docs::{CRATES_IO_BASE_URL, DOCS_RS_BASE_URL, Server};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:8000";

#[derive(Debug, Parser)]
#[command(name = "mcp-rust-docs", version, about, long_about = None)]
struct Cli {
    /// Upstream crates.io base URL. Useful for pointing at a registry
    /// mirror, a wiremock fixture, or a logging proxy.
    #[arg(
        long,
        env = "MCP_CRATES_IO_BASE_URL",
        default_value = CRATES_IO_BASE_URL,
        global = true,
    )]
    crates_io_base_url: String,

    /// Upstream docs.rs base URL. Same use cases as the crates.io
    /// override — wiremock fixtures, mirrors, proxies.
    #[arg(
        long,
        env = "MCP_DOCS_RS_BASE_URL",
        default_value = DOCS_RS_BASE_URL,
        global = true,
    )]
    docs_rs_base_url: String,

    #[command(subcommand)]
    transport: Transport,
}

#[derive(Debug, Subcommand)]
enum Transport {
    /// Serve MCP over stdin/stdout (the transport an MCP host launches
    /// the binary with directly).
    Stdio,

    /// Serve MCP over streamable HTTP at `/mcp`.
    Http(HttpArgs),
}

#[derive(Debug, Args)]
struct HttpArgs {
    /// TCP address to bind the HTTP listener to.
    #[arg(
        long,
        env = "MCP_BIND_ADDRESS",
        default_value = DEFAULT_BIND_ADDRESS,
    )]
    bind: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.transport {
        Transport::Stdio => run_stdio(&cli.crates_io_base_url, &cli.docs_rs_base_url).await,
        Transport::Http(args) => {
            run_http(&cli.crates_io_base_url, &cli.docs_rs_base_url, &args.bind).await
        }
    }
}

/// Initialize tracing for the stdio transport.
///
/// stdio servers MUST NOT write anything except JSON-RPC to stdout —
/// the MCP host is parsing every byte. So tracing goes to stderr only,
/// with ANSI escape codes stripped (host log viewers usually render
/// raw text).
fn init_stdio_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

fn init_http_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

async fn run_stdio(crates_io_base_url: &str, docs_rs_base_url: &str) -> anyhow::Result<()> {
    init_stdio_tracing();

    tracing::info!(
        %crates_io_base_url,
        %docs_rs_base_url,
        "starting mcp-rust-docs over stdio",
    );

    let server = Server::builder()
        .crates_io_base_url(crates_io_base_url.to_string())
        .docs_rs_base_url(docs_rs_base_url.to_string())
        .build()?;

    let service = server.serve(stdio()).await.inspect_err(|err| {
        tracing::error!(error = ?err, "failed to start MCP server");
    })?;

    service.waiting().await?;
    Ok(())
}

async fn run_http(
    crates_io_base_url: &str,
    docs_rs_base_url: &str,
    bind_address: &str,
) -> anyhow::Result<()> {
    init_http_tracing();

    let cancellation = tokio_util::sync::CancellationToken::new();

    // Build the Server (and its `reqwest::Client`) ONCE, then clone
    // into the per-session factory closure. Server is cheap to clone
    // (use cases are behind `Arc`s), so every session reuses the same
    // HTTP client and connection pool. Calling `Server::builder().build()`
    // inside the factory instead would spin up a fresh client per
    // session and waste the connection pool.
    let server_template = Server::builder()
        .crates_io_base_url(crates_io_base_url.to_string())
        .docs_rs_base_url(docs_rs_base_url.to_string())
        .build()?;

    let service = StreamableHttpService::new(
        move || Ok(server_template.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(cancellation.child_token()),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    tracing::info!(
        %bind_address,
        %crates_io_base_url,
        %docs_rs_base_url,
        "mcp-rust-docs listening at /mcp",
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("received Ctrl+C, shutting down");
            cancellation.cancel();
        })
        .await?;

    Ok(())
}
