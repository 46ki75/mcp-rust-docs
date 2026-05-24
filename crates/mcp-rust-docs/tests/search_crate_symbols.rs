//! Hermetic integration tests for the `search_crate_symbols` tool.
//!
//! Same pattern as `search_crates.rs` and `get_crate_docs.rs`: drive
//! the server via a `tokio::io::duplex` pipe and serve a fake
//! `all.html` from a `wiremock::MockServer`.

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

/// Tiny rustdoc-shaped all.html fixture covering three kinds.
const ALL_HTML: &str = r#"<html><body><main>
    <h3 id="structs">Structs</h3>
    <ul>
        <li><a href="struct.Error.html">Error</a></li>
        <li><a href="de/value/struct.U8Deserializer.html">de::value::U8Deserializer</a></li>
    </ul>
    <h3 id="traits">Traits</h3>
    <ul>
        <li><a href="trait.Deserialize.html">Deserialize</a></li>
    </ul>
    <h3 id="derives">Derive Macros</h3>
    <ul>
        <li><a href="derive.Deserialize.html">Deserialize</a></li>
    </ul>
</main></body></html>"#;

#[tokio::test]
async fn list_tools_advertises_search_crate_symbols() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let tools = client.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "search_crate_symbols"),
        "search_crate_symbols not advertised: {tools:?}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_symbols_returns_matched_items_with_composable_paths() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/serde/latest/serde/all.html"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALL_HTML))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_symbols").with_arguments(args(json!({
                "crate_name": "serde",
                "query": "Deserializer",
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
    assert_eq!(parsed["crate_name"], "serde");
    assert_eq!(parsed["total_matched"], 1);
    assert_eq!(parsed["truncated"], false);
    let items = parsed["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "struct");
    assert_eq!(items[0]["name"], "de::value::U8Deserializer");
    assert_eq!(items[0]["path"], "de/value/struct.U8Deserializer.html");

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_symbols_filters_by_kind() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/serde/latest/serde/all.html"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALL_HTML))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_symbols").with_arguments(args(json!({
                "crate_name": "serde",
                "query": "deserialize",
                "kinds": ["derive"],
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
    assert_eq!(parsed["total_matched"], 1);
    let items = parsed["items"].as_array().expect("items");
    assert_eq!(items[0]["kind"], "derive");

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
async fn search_crate_symbols_reports_404_as_not_found() -> anyhow::Result<()> {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/nonexistent/latest/nonexistent/all.html"))
        .respond_with(ResponseTemplate::new(404).set_body_string("missing"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder().docs_rs_base_url(mock.uri()).build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_symbols")
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
        text.starts_with("Not found:"),
        "expected not-found prefix, got: {text}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}
