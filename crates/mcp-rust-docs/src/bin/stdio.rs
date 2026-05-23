use mcp_rust_docs::Server;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("starting mcp-rust-docs over stdio");

    let mut builder = Server::builder();
    if let Ok(base_url) = std::env::var("MCP_CRATES_IO_BASE_URL") {
        tracing::info!(%base_url, "overriding crates.io base URL");
        builder = builder.base_url(base_url);
    }

    let service = builder.build()?.serve(stdio()).await.inspect_err(|err| {
        tracing::error!(error = ?err, "failed to start MCP server");
    })?;

    service.waiting().await?;
    Ok(())
}
