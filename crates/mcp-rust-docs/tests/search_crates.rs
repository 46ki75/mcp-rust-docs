use std::time::Duration;

use mcp_rust_docs::Server;
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default, Clone)]
struct TestClient;

impl ClientHandler for TestClient {}

async fn spawn(
    server: Server,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let (server_io, client_io) = tokio::io::duplex(4096);

    let server_handle = tokio::spawn(async move {
        let svc = server.serve(server_io).await?;
        svc.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient
        .serve(client_io)
        .await
        .expect("client failed to connect");

    (client, server_handle)
}

fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

fn fixture_body() -> serde_json::Value {
    json!({
        "crates": [
            {
                "id": "tokio",
                "name": "tokio",
                "description": "An event-driven, non-blocking I/O platform.",
                "max_version": "1.40.0",
                "max_stable_version": "1.40.0",
                "downloads": 500_000_000u64,
                "recent_downloads": 12_345_678u64,
                "documentation": "https://docs.rs/tokio",
                "homepage": "https://tokio.rs",
                "repository": "https://github.com/tokio-rs/tokio",
                "updated_at": "2025-01-01T00:00:00Z"
            },
            {
                "id": "tokio-util",
                "name": "tokio-util",
                "description": "Additional utilities for working with Tokio.",
                "max_version": "0.7.10",
                "max_stable_version": "0.7.10",
                "downloads": 100_000_000u64,
                "recent_downloads": 1_234_567u64,
                "documentation": serde_json::Value::Null,
                "homepage": serde_json::Value::Null,
                "repository": "https://github.com/tokio-rs/tokio",
                "updated_at": "2025-01-02T00:00:00Z"
            }
        ],
        "meta": { "total": 42 }
    })
}

#[tokio::test]
async fn search_crates_returns_parsed_results() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .and(query_param("q", "tokio"))
        .and(query_param("per_page", "2"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_body()))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().crates_io_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "tokio", "per_page": 2 }))),
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
    assert_eq!(parsed["total"], 42);
    assert_eq!(parsed["page"], 1);
    assert_eq!(parsed["per_page"], 2);
    assert_eq!(parsed["crates"][0]["name"], "tokio");
    assert_eq!(parsed["crates"][0]["version"], "1.40.0");
    assert_eq!(parsed["crates"][0]["downloads"], 500_000_000u64);
    assert_eq!(parsed["crates"][1]["name"], "tokio-util");
    assert!(parsed["crates"][1].get("homepage").is_none());

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crates_reports_upstream_http_errors() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .respond_with(ResponseTemplate::new(503).set_body_string("temporarily unavailable"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().crates_io_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "anything" }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true), "expected tool to report error");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(text.contains("503"), "error text missing status: {text}");

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crates_aborts_when_upstream_exceeds_http_timeout() -> anyhow::Result<()> {
    // Without a default `reqwest` timeout, a stuck upstream socket
    // hangs the streamable-HTTP transport indefinitely (no per-call
    // deadline at the protocol layer either). Pin the contract: a
    // tight builder-supplied timeout MUST fire and surface as a tool
    // error rather than blocking the handler.
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture_body())
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&mock)
        .await;

    let server = Server::builder()
        .crates_io_base_url(mock.uri())
        .http_timeout(Duration::from_millis(100))
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "tokio" }))),
        )
        .await?;

    assert_eq!(
        result.is_error,
        Some(true),
        "tool should error rather than hang: {result:?}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[test]
fn default_http_timeout_is_set_and_reasonable() {
    // Pins that the default-builder branch installs a non-zero, not
    // absurdly large timeout. The integration test above only proves
    // an explicit override works; this guards against a refactor that
    // silently drops the default.
    let timeout = mcp_rust_docs::router::DEFAULT_HTTP_TIMEOUT;
    assert!(timeout > Duration::from_secs(0));
    assert!(timeout < Duration::from_secs(120));

    let connect = mcp_rust_docs::router::DEFAULT_HTTP_CONNECT_TIMEOUT;
    assert!(connect > Duration::from_secs(0));
    assert!(connect <= timeout);
}

#[tokio::test]
async fn list_tools_advertises_search_crates() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    let server = Server::builder().crates_io_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let tools = client.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "search_crates"),
        "search_crates not advertised: {tools:?}"
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}
