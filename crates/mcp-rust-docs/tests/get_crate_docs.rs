//! Hermetic integration tests for the `get_crate_docs` tool.
//!
//! Same pattern as `search_crates.rs`: drive the server via a
//! `tokio::io::duplex` pipe so no OS-level I/O is involved, and
//! point the docs.rs upstream at a `wiremock::MockServer` so the
//! HTTP shape is exercised end-to-end without leaving the test
//! process.

use mcp_rust_docs::Server;
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use wiremock::matchers::{method, path};
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

/// Minimal rustdoc-shaped HTML — enough nesting to confirm tags are
/// stripped and headings/paragraphs round-trip through html2md.
fn fixture_html() -> &'static str {
    r#"<!DOCTYPE html><html><body>
        <main>
            <h1>Module tokio</h1>
            <p>An <strong>async</strong> runtime.</p>
            <p>See <a href="task/struct.JoinHandle.html">JoinHandle</a>.</p>
        </main>
    </body></html>"#
}

#[tokio::test]
async fn list_tools_advertises_get_crate_docs() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let tools = client.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "get_crate_docs"),
        "get_crate_docs not advertised: {tools:?}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn get_crate_docs_returns_markdown_for_root() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/tokio/latest/tokio/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture_html()))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs")
                .with_arguments(args(json!({ "crate_name": "tokio" }))),
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
    assert_eq!(parsed["crate_name"], "tokio");
    let md = parsed["markdown"].as_str().expect("markdown string");
    assert!(md.contains("Module tokio"), "missing heading: {md}");
    assert!(md.contains("async"), "missing strong text: {md}");
    assert!(
        !md.contains("<strong>") && !md.contains("<a "),
        "html should be stripped: {md}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn get_crate_docs_translates_hyphens_in_lib_name() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/tokio-util/0.7.10/tokio_util/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<p>ok</p>"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs").with_arguments(args(json!({
                "crate_name": "tokio-util",
                "version": "0.7.10",
            }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "tool returned error: {result:?}"
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn get_crate_docs_appends_path_tail() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/tokio/latest/tokio/task/struct.JoinHandle.html"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<h1>JoinHandle</h1>"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs").with_arguments(args(json!({
                "crate_name": "tokio",
                "path": "task/struct.JoinHandle.html",
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
    let md = parsed["markdown"].as_str().expect("markdown string");
    assert!(md.contains("JoinHandle"), "missing item name: {md}");

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn get_crate_docs_reports_404_as_not_found_error() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/nonexistent/latest/nonexistent/"))
        .respond_with(ResponseTemplate::new(404).set_body_string("missing"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs")
                .with_arguments(args(json!({ "crate_name": "nonexistent" }))),
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
        text.starts_with("Not found:") && text.contains("404"),
        "expected not-found prefix, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn get_crate_docs_rejects_traversal_paths_without_calling_upstream() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    // No mock mounted — any request the server makes will get a 404,
    // which would surface as `Not found:` rather than `Invalid request:`.
    // The assertion below pins down which error class fired.
    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs").with_arguments(args(json!({
                "crate_name": "tokio",
                "path": "../../../etc/passwd",
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
        "expected validation error, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}
