//! Hermetic integration tests for the `get_crate_docs` tool.
//!
//! Same pattern as `search_crates.rs`: drive the server via a
//! `tokio::io::duplex` pipe so no OS-level I/O is involved, and
//! point the docs.rs upstream at a `wiremock::MockServer` so the
//! HTTP shape is exercised end-to-end without leaving the test
//! process.
//!
//! Every test points BOTH `docs_rs_base_url` and `crates_io_base_url`
//! at the same wiremock instance. `get_crate_docs` now bundles
//! crate metadata fetched from crates.io on root calls (no `path`),
//! so an un-overridden builder would leak real-network traffic.
//! Tests that don't care about metadata leave the crates.io routes
//! unmocked — those requests 404, the response carries a populated
//! `metadata_error`, and the docs payload (the load-bearing thing)
//! still ships. Tests that want to assert metadata mount the
//! corresponding `/api/v1/crates/...` routes explicitly.

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
    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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

/// Crate-root call: the response must carry the metadata block
/// (versions / features / dependencies) fetched in parallel from
/// crates.io. Pins both the parallel composition and the wire shape
/// the agent will see.
#[tokio::test]
async fn get_crate_docs_attaches_metadata_on_root_call() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/tokio/latest/tokio/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture_html()))
        .expect(1)
        .mount(&mock)
        .await;
    // Per-crate aggregate: one stable version with one feature.
    Mock::given(method("GET"))
        .and(path("/api/v1/crates/tokio"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "crate": {
                "name": "tokio",
                "max_version": "1.40.0",
                "max_stable_version": "1.40.0"
            },
            "versions": [
                {
                    "num": "1.40.0",
                    "yanked": false,
                    "created_at": "2025-01-01T00:00:00Z",
                    "features": { "default": ["rt"], "full": ["rt", "macros"] }
                }
            ]
        })))
        .expect(1)
        .mount(&mock)
        .await;
    // Per-version dependencies: one normal, one dev.
    Mock::given(method("GET"))
        .and(path("/api/v1/crates/tokio/1.40.0/dependencies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dependencies": [
                { "crate_id": "bytes", "req": "^1.0", "kind": "normal", "optional": false },
                { "crate_id": "tokio-test", "req": "^0.4", "kind": "dev", "optional": false }
            ]
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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
    let metadata = parsed["metadata"]
        .as_object()
        .unwrap_or_else(|| panic!("metadata missing from response: {parsed}"));

    assert_eq!(metadata["crate_name"], "tokio");
    assert_eq!(metadata["resolved_version"], "1.40.0");
    assert_eq!(metadata["resolved_version_yanked"], false);

    let versions = metadata["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["num"], "1.40.0");

    let features = metadata["features"].as_object().expect("features object");
    assert!(features.contains_key("default"));
    assert!(features.contains_key("full"));

    let deps = metadata["dependencies"]
        .as_object()
        .expect("dependencies object");
    assert_eq!(deps["runtime_count"], 1);
    assert_eq!(deps["dev_count"], 1);
    assert_eq!(deps["build_count"], 0);
    let runtime = deps["runtime"].as_array().expect("runtime list");
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0]["name"], "bytes");

    // metadata_error must be absent when the fetch succeeded.
    assert!(
        parsed.get("metadata_error").is_none() || parsed["metadata_error"].is_null(),
        "metadata_error should be absent on success: {parsed}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

/// Drill-down call (with `path`): the metadata block must NOT be
/// fetched — the agent already has it from the root call, and
/// re-fetching per page wastes a crates.io round-trip. The
/// `.expect(0)` on the crates.io routes locks this in.
#[tokio::test]
async fn get_crate_docs_omits_metadata_when_path_is_set() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/tokio/latest/tokio/task/struct.JoinHandle.html"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<h1>JoinHandle</h1>"))
        .expect(1)
        .mount(&mock)
        .await;
    // The metadata fetch must not fire on drill-down. If the code
    // regresses and starts fetching unconditionally, these
    // expectations will fail at mock teardown.
    Mock::given(method("GET"))
        .and(path("/api/v1/crates/tokio"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"^/api/v1/crates/tokio/.+/dependencies$",
        ))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs").with_arguments(args(json!({
                "crate_name": "tokio",
                "path": "task/struct.JoinHandle.html",
            }))),
        )
        .await?;

    assert!(!result.is_error.unwrap_or(false));
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert!(
        parsed.get("metadata").is_none() || parsed["metadata"].is_null(),
        "drill-down responses must omit metadata: {parsed}",
    );
    assert!(
        parsed.get("metadata_error").is_none() || parsed["metadata_error"].is_null(),
        "drill-down responses must omit metadata_error too: {parsed}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

/// Best-effort metadata: a crates.io failure on a root call must not
/// kill the docs payload. The response is non-error, `markdown` is
/// present, and `metadata_error` surfaces the underlying cause so the
/// caller can decide whether to retry.
#[tokio::test]
async fn get_crate_docs_returns_docs_when_metadata_fetch_fails() -> anyhow::Result<()> {
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/tokio/latest/tokio/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture_html()))
        .expect(1)
        .mount(&mock)
        .await;
    // crates.io aggregate endpoint craters with a 503. Metadata
    // should fail gracefully; docs should still ship.
    Mock::given(method("GET"))
        .and(path("/api/v1/crates/tokio"))
        .respond_with(ResponseTemplate::new(503).set_body_string("registry down"))
        .expect(1)
        .mount(&mock)
        .await;

    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs")
                .with_arguments(args(json!({ "crate_name": "tokio" }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "metadata failure must not fail the call: {result:?}",
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    let md = parsed["markdown"]
        .as_str()
        .expect("markdown string must be present");
    assert!(
        !md.is_empty(),
        "docs markdown must still be present: {parsed}"
    );
    assert!(
        parsed.get("metadata").is_none() || parsed["metadata"].is_null(),
        "metadata must be absent when fetch failed: {parsed}",
    );
    let err = parsed["metadata_error"]
        .as_str()
        .unwrap_or_else(|| panic!("metadata_error missing: {parsed}"));
    assert!(
        err.contains("503"),
        "metadata_error should name the 503 cause, got: {err}",
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
    let server = Server::builder()
        .docs_rs_base_url(mock.uri())
        .crates_io_base_url(mock.uri())
        .build()?;
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
