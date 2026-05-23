//! End-to-end tests over the streamable-HTTP transport.
//!
//! Spins up the same `StreamableHttpService` that `bin/http.rs` mounts, but
//! bound to an ephemeral TCP port and pointed at a wiremock-backed
//! upstream so no traffic reaches the real crates.io.

use mcp_rust_docs::Server;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DOCS_FIXTURE_HTML: &str =
    "<main><h1>Module serde</h1><p>Serialize and deserialize.</p></main>";

#[derive(Default, Clone)]
struct TestClient;

impl ClientHandler for TestClient {}

struct HttpServerHandle {
    base_url: String,
    cancellation: CancellationToken,
    join: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl HttpServerHandle {
    async fn shutdown(self) {
        self.cancellation.cancel();
        let _ = self.join.await;
    }
}

async fn spawn_http_server(server_template: Server) -> anyhow::Result<HttpServerHandle> {
    let cancellation = CancellationToken::new();

    let service = StreamableHttpService::new(
        move || Ok(server_template.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(cancellation.child_token()),
    );

    let router = axum::Router::new().nest_service("/mcp", service);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let base_url = format!("http://127.0.0.1:{port}/mcp");

    let shutdown_token = cancellation.clone();
    let join = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown_token.cancelled().await })
            .await
    });

    Ok(HttpServerHandle {
        base_url,
        cancellation,
        join,
    })
}

fn fixture_body() -> serde_json::Value {
    json!({
        "crates": [
            {
                "id": "serde",
                "name": "serde",
                "description": "A generic serialization/deserialization framework.",
                "max_version": "1.0.200",
                "max_stable_version": "1.0.200",
                "downloads": 800_000_000u64,
                "recent_downloads": 20_000_000u64,
                "documentation": "https://docs.rs/serde",
                "homepage": "https://serde.rs",
                "repository": "https://github.com/serde-rs/serde",
                "updated_at": "2025-02-01T00:00:00Z"
            }
        ],
        "meta": { "total": 1 }
    })
}

fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

#[tokio::test]
async fn http_client_can_list_and_call_search_crates() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .and(query_param("q", "serde"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_body()))
        .expect(1)
        .mount(&mock)
        .await;

    let server_template = Server::builder().crates_io_base_url(mock.uri()).build()?;
    let http = spawn_http_server(server_template).await?;

    let transport = StreamableHttpClientTransport::from_uri(http.base_url.clone());
    let client = TestClient.serve(transport).await?;

    let tools = client.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "search_crates"),
        "search_crates not advertised over HTTP: {tools:?}",
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "serde" }))),
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
    assert_eq!(parsed["crates"][0]["name"], "serde");
    assert_eq!(parsed["crates"][0]["version"], "1.0.200");

    client.cancel().await?;
    http.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn http_client_can_call_get_crate_docs() -> anyhow::Result<()> {
    // Two mocks on the same server: crates.io path stays unmounted
    // (this test never calls it), docs.rs path serves a tiny fixture.
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/serde/latest/serde/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(DOCS_FIXTURE_HTML))
        .expect(1)
        .mount(&mock)
        .await;

    let server_template = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
    let http = spawn_http_server(server_template).await?;

    let transport = StreamableHttpClientTransport::from_uri(http.base_url.clone());
    let client = TestClient.serve(transport).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs")
                .with_arguments(args(json!({ "crate_name": "serde" }))),
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
    assert_eq!(parsed["crate_name"], "serde");
    let md = parsed["markdown"].as_str().expect("markdown string");
    assert!(md.contains("Module serde"));

    client.cancel().await?;
    http.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn http_client_can_call_search_crate_symbols() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/serde/latest/serde/all.html"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<main>
                <h3 id="traits">Traits</h3>
                <ul><li><a href="trait.Deserialize.html">Deserialize</a></li></ul>
            </main>"#,
        ))
        .expect(1)
        .mount(&mock)
        .await;

    let server_template = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let http = spawn_http_server(server_template).await?;

    let transport = StreamableHttpClientTransport::from_uri(http.base_url.clone());
    let client = TestClient.serve(transport).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_symbols").with_arguments(args(json!({
                "crate_name": "serde",
                "query": "Deserialize",
            }))),
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
    assert_eq!(parsed["items"][0]["name"], "Deserialize");
    assert_eq!(parsed["items"][0]["path"], "trait.Deserialize.html");

    client.cancel().await?;
    http.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn http_client_surfaces_upstream_errors_as_tool_errors() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
        .expect(1)
        .mount(&mock)
        .await;

    let server_template = Server::builder().crates_io_base_url(mock.uri()).build()?;
    let http = spawn_http_server(server_template).await?;

    let transport = StreamableHttpClientTransport::from_uri(http.base_url.clone());
    let client = TestClient.serve(transport).await?;

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
    assert!(text.contains("502"), "error text missing status: {text}");

    client.cancel().await?;
    http.shutdown().await;
    Ok(())
}
