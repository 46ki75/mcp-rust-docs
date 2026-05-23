//! End-to-end tests over the real stdio transport.
//!
//! Spawns the compiled `mcp-rust-docs` binary with the `stdio`
//! subcommand and drives it via rmcp's `TokioChildProcess` transport.
//! The binary reads `MCP_CRATES_IO_BASE_URL` from the env, so each
//! test points it at a wiremock-backed upstream — no traffic reaches
//! real crates.io.
//!
//! These complement `e2e_http.rs` (which exercises the streamable-HTTP
//! transport in-process) and `search_crates.rs` (which uses an in-memory
//! duplex pipe). Here the bytes actually round-trip through the OS
//! pipe and the bundled binary, so this is the closest thing in CI to
//! how a real MCP host launches the server.

use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default, Clone)]
struct TestClient;

impl ClientHandler for TestClient {}

fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

fn fixture_body() -> serde_json::Value {
    json!({
        "crates": [
            {
                "id": "anyhow",
                "name": "anyhow",
                "description": "Flexible concrete Error type built on std::error::Error.",
                "max_version": "1.0.80",
                "max_stable_version": "1.0.80",
                "downloads": 400_000_000u64,
                "recent_downloads": 8_000_000u64,
                "documentation": "https://docs.rs/anyhow",
                "homepage": serde_json::Value::Null,
                "repository": "https://github.com/dtolnay/anyhow",
                "updated_at": "2025-03-01T00:00:00Z"
            }
        ],
        "meta": { "total": 1 }
    })
}

/// Spawn the bundled stdio binary with `MCP_CRATES_IO_BASE_URL` pointed
/// at the given upstream URL, and return a connected MCP client.
async fn spawn_stdio_child(
    upstream_base_url: &str,
) -> anyhow::Result<rmcp::service::RunningService<rmcp::RoleClient, TestClient>> {
    // Cargo sets this for every integration test in a crate that has a
    // matching `[[bin]]`. No need to invoke `cargo run` ourselves —
    // cargo has already built (and rebuilt-if-needed) the binary by the
    // time this test starts.
    let bin = env!("CARGO_BIN_EXE_mcp-rust-docs");
    let upstream = upstream_base_url.to_string();

    let command = tokio::process::Command::new(bin).configure(|cmd| {
        cmd.arg("stdio")
            .env("MCP_CRATES_IO_BASE_URL", &upstream)
            // Keep the child's stderr quiet; tracing at info level would
            // otherwise spam the test output.
            .env("RUST_LOG", "error");
    });

    let transport = TokioChildProcess::new(command)?;
    let client = TestClient.serve(transport).await?;
    Ok(client)
}

#[tokio::test]
async fn stdio_child_lists_search_crates_tool() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    let client = spawn_stdio_child(&mock.uri()).await?;

    let tools = client.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "search_crates"),
        "search_crates not advertised by stdio child: {tools:?}",
    );

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn stdio_child_returns_parsed_search_results() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .and(query_param("q", "anyhow"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_body()))
        .expect(1)
        .mount(&mock)
        .await;

    let client = spawn_stdio_child(&mock.uri()).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "anyhow" }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "tool returned error: {result:?}"
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(parsed["total"], 1);
    assert_eq!(parsed["crates"][0]["name"], "anyhow");
    assert_eq!(parsed["crates"][0]["version"], "1.0.80");

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn stdio_child_surfaces_upstream_errors_as_tool_errors() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream meltdown"))
        .expect(1)
        .mount(&mock)
        .await;

    let client = spawn_stdio_child(&mock.uri()).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "anything" }))),
        )
        .await?;

    assert_eq!(
        result.is_error,
        Some(true),
        "expected tool to report upstream failure"
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(text.contains("500"), "error text missing status: {text}");

    client.cancel().await?;
    Ok(())
}
