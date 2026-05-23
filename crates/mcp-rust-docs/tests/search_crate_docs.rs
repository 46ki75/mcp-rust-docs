//! Hermetic integration tests for the `search_crate_docs` tool.
//!
//! Drives the server via `tokio::io::duplex` and serves a real (but
//! version-pinned) anyhow rustdoc JSON fixture, zstd-compressed,
//! through `wiremock`. This proves the full pipeline — URL assembly,
//! HTTP fetch, zstd decompression, rustdoc-types deserialization,
//! filtering, ranking, snippet generation — without touching the
//! network.

use mcp_rust_docs::Server;
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default, Clone)]
struct TestClient;

impl ClientHandler for TestClient {}

/// The anyhow crate's zstd-compressed rustdoc JSON, captured against
/// the same `rustdoc-types` major version this crate depends on.
/// Refresh if format_version drifts and the JSON stops deserializing.
const ANYHOW_JSON_ZST: &[u8] = include_bytes!("fixtures/anyhow_rustdoc.json.zst");

async fn spawn(
    server: Server,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);

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
async fn list_tools_advertises_search_crate_docs() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let tools = client.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "search_crate_docs"),
        "search_crate_docs not advertised: {tools:?}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_docs_returns_hits_with_snippet() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/crate/anyhow/latest/json.zst"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ANYHOW_JSON_ZST.to_vec(), "application/zstd"),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .build()?;
    let (client, server_handle) = spawn(server).await;

    // "error" is a near-certain match in the anyhow docs (the crate
    // exists to wrap errors). If this regresses, the JSON deserialized
    // but the doc-comment walk has stopped picking up trait/struct docs.
    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                "crate_name": "anyhow",
                "query": "error",
                "limit": 5,
            }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "tool returned error: {result:?}",
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(parsed["crate_name"], "anyhow");
    let total: u64 = parsed["total_matched"].as_u64().expect("total_matched int");
    assert!(total > 0, "expected at least one hit: {parsed}");

    let items = parsed["items"].as_array().expect("items array");
    assert!(!items.is_empty(), "no items returned: {parsed}");
    assert!(
        items.len() <= 5,
        "limit not respected: returned {} items",
        items.len(),
    );

    // Every hit must carry the four contract fields.
    for item in items {
        assert!(item["kind"].is_string(), "missing kind: {item}");
        assert!(item["name"].is_string(), "missing name: {item}");
        assert!(item["path"].is_string(), "missing path: {item}");
        assert!(item["snippet"].is_string(), "missing snippet: {item}");
        let snippet = item["snippet"].as_str().expect("snippet string");
        // The snippet should contain the query (case-insensitive).
        assert!(
            snippet.to_lowercase().contains("error"),
            "snippet missing query: {snippet}",
        );
    }

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_docs_filters_by_kind() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/crate/anyhow/latest/json.zst"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ANYHOW_JSON_ZST.to_vec(), "application/zstd"),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                "crate_name": "anyhow",
                "query": "error",
                "kinds": ["macro"],
                "limit": 50,
            }))),
        )
        .await?;

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    let items = parsed["items"].as_array().expect("items");
    for item in items {
        assert_eq!(
            item["kind"], "macro",
            "kind filter leaked non-macro: {item}"
        );
    }

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_docs_rejects_empty_query_with_invalid_request() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    // No mock mount — an empty query should be rejected before any
    // HTTP call. If we accidentally hit the upstream, wiremock will
    // return 404 by default and the assertion below would catch it.
    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                "crate_name": "anyhow",
                "query": "   ",
            }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(
        text.starts_with("Invalid request:"),
        "expected invalid-request prefix, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_docs_reports_404_as_not_found() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/crate/nonexistent/latest/json.zst"))
        .respond_with(ResponseTemplate::new(404).set_body_string("missing"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                "crate_name": "nonexistent",
                "query": "pin",
            }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(
        text.starts_with("Not found:"),
        "expected not-found prefix, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_docs_reports_format_version_mismatch() -> anyhow::Result<()> {
    // Take the real anyhow fixture, decompress it, swap only
    // `format_version` to a value the repo cannot understand, then
    // recompress. This way every other field is still valid rustdoc
    // JSON and we exercise the dedicated format-version check rather
    // than tripping over a generic parse failure on something else.
    let mutated = mutate_format_version(ANYHOW_JSON_ZST, 99_999_999);

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/crate/anyhow/latest/json.zst"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(mutated, "application/zstd"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .docs_rs_cache_enabled(false)
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                "crate_name": "anyhow",
                "query": "pin",
            }))),
        )
        .await?;

    assert_eq!(result.is_error, Some(true));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    // The tool error formatter wraps repository errors with
    // "Upstream failure:". The format-version mismatch's Display
    // message names both versions so the user knows which side is
    // out of date.
    assert!(
        text.contains("format_version") && text.contains("99999999"),
        "expected format-version diagnostic naming both versions, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

/// Decompress a rustdoc-JSON-zst payload, set `format_version` to
/// `new_value`, and recompress. Used by the mismatch test so the
/// fixture stays in sync with the rest of the JSON schema.
fn mutate_format_version(compressed: &[u8], new_value: u32) -> Vec<u8> {
    use std::io::Read;

    let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed).expect("zstd decode");
    let mut decompressed = Vec::with_capacity(compressed.len() * 5);
    decoder
        .read_to_end(&mut decompressed)
        .expect("zstd read_to_end");
    let mut value: serde_json::Value =
        serde_json::from_slice(&decompressed).expect("parse anyhow fixture");
    value["format_version"] = serde_json::Value::from(new_value);
    let reserialized = serde_json::to_vec(&value).expect("serialize");
    encode_zstd(&reserialized)
}

/// Compress `data` with zstd into a single frame using ruzstd's
/// pure-Rust encoder. We don't pull in the libzstd C binding just for
/// tests.
fn encode_zstd(data: &[u8]) -> Vec<u8> {
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};
    compress_to_vec(data, CompressionLevel::Fastest)
}

/// End-to-end proof that the rustdoc-JSON cache short-circuits the
/// second call. Uses `.expect(1)` on the wiremock mount: if the cache
/// silently regresses to pass-through, the wiremock teardown would fail
/// with "expected 1, got 2". Cache is left at its default (enabled).
#[tokio::test]
async fn second_search_crate_docs_call_is_served_from_cache() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/crate/anyhow/latest/json.zst"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ANYHOW_JSON_ZST.to_vec(), "application/zstd"),
        )
        .expect(1)
        .mount(&mock)
        .await;

    // Cache enabled — that's what we're testing.
    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    for query in ["error", "Result"] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("search_crate_docs").with_arguments(args(json!({
                    "crate_name": "anyhow",
                    "query": query,
                    "limit": 1,
                }))),
            )
            .await?;
        assert!(
            !result.is_error.unwrap_or(false),
            "tool returned error for query {query:?}: {result:?}",
        );
    }

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}
