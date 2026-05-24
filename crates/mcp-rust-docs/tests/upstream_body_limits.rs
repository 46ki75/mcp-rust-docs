//! Pins the body-size cap on both upstream HTTP repositories.
//!
//! `DEFAULT_HTTP_TIMEOUT` mitigates a slow drip of gigabytes, but a
//! cooperative high-bandwidth upstream that streams the full body
//! within the timeout would still exhaust memory if reads are
//! unbounded. The cap exists so a misbehaving (or malicious) mirror
//! cannot pin down the process by pushing a huge body.

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
    let (server_io, client_io) = tokio::io::duplex(16 * 1024);

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

#[tokio::test]
async fn search_crates_rejects_oversized_response_body() -> anyhow::Result<()> {
    // crates.io's real search response is <1 MB even for prolific
    // queries. A response 100x larger than what the body cap allows
    // must surface as a tool error rather than being buffered into
    // memory.
    let mock = MockServer::start().await;
    let oversized = vec![b'x'; 64 * 1024]; // 64 KB
    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .and(query_param("q", "tokio"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(oversized))
        .mount(&mock)
        .await;

    let server = Server::builder()
        .crates_io_base_url(mock.uri())
        .upstream_body_size_limit(4 * 1024) // 4 KB cap
        .http_timeout(Duration::from_secs(5))
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "tokio" }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true), "expected tool error");
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(
        text.contains("exceed") || text.contains("cap") || text.contains("too large"),
        "error should mention size cap, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn get_crate_docs_rejects_oversized_html_body() -> anyhow::Result<()> {
    // docs.rs's largest HTML doc page sits well under 5 MB. A body
    // larger than the configured cap must error rather than allocate.
    let mock = MockServer::start().await;
    let oversized = vec![b'x'; 64 * 1024]; // 64 KB
    Mock::given(method("GET"))
        .and(path("/tokio/latest/tokio/"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(oversized))
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .upstream_body_size_limit(4 * 1024) // 4 KB cap
        .http_timeout(Duration::from_secs(5))
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs")
                .with_arguments(args(json!({ "crate_name": "tokio" }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true), "expected tool error");
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(
        text.contains("exceed") || text.contains("cap") || text.contains("too large"),
        "error should mention size cap, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_docs_rejects_oversized_rustdoc_json_body() -> anyhow::Result<()> {
    // The zstd-compressed rustdoc JSON path is the one most likely to
    // approach the body cap in production (tens of MB for large
    // crates). The use case's format-version fallback chain
    // short-circuits on any non-404 repository error, so a single
    // oversized response on the first attempted format must surface
    // as a tool error rather than triggering further upstream calls
    // or buffering the body in memory.
    let mock = MockServer::start().await;
    let oversized = vec![b'x'; 64 * 1024]; // 64 KB
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"^/crate/anyhow/latest/json/\d+\.zst$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(oversized, "application/zstd"))
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .upstream_body_size_limit(4 * 1024) // 4 KB cap
        .http_timeout(Duration::from_secs(5))
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                "crate_name": "anyhow",
                "query": "error",
            }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true), "expected tool error");
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(
        text.contains("exceed") || text.contains("cap") || text.contains("too large"),
        "error should mention size cap, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[test]
fn default_upstream_body_size_limit_is_set_and_reasonable() {
    let limit = mcp_rust_docs::router::DEFAULT_UPSTREAM_BODY_BYTES;
    // Must be large enough for real rustdoc-JSON payloads (tens of
    // MB compressed) but small enough that a single response can't
    // OOM a normal process.
    assert!(limit >= 64 * 1024 * 1024);
    assert!(limit <= 512 * 1024 * 1024);
}
