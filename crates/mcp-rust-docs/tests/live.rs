//! Live tests — hit the real crates.io API.
//!
//! Skipped by default via `#[ignore]`. Run with `just test-live`
//! (or `cargo test -- --ignored`). Failures here may reflect upstream
//! state (network, rate limits, registry changes) rather than this diff,
//! so per the org standards they do not gate PR merges.

use mcp_rust_docs::Server;
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use serde_json::json;

#[derive(Default, Clone)]
struct TestClient;

impl ClientHandler for TestClient {}

fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

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

#[tokio::test]
#[ignore = "live: hits real crates.io API"]
async fn live_search_returns_serde_from_crates_io() -> anyhow::Result<()> {
    let server = Server::new()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "serde", "per_page": 5 }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "live tool returned error: {result:?}"
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert!(
        parsed["total"].as_u64().unwrap_or(0) > 0,
        "expected non-zero total for `serde` query: {parsed}"
    );

    let crates = parsed["crates"].as_array().expect("crates array");
    assert!(!crates.is_empty(), "no crates returned: {parsed}");

    // `serde` itself should be in the top hits for a `serde` query — if
    // it isn't, the response shape parsing is almost certainly wrong.
    let serde_crate = crates
        .iter()
        .find(|c| c["name"] == "serde")
        .unwrap_or_else(|| panic!("`serde` not found in results: {parsed}"));

    assert!(
        serde_crate["version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "`serde` missing version field: {serde_crate}"
    );
    assert!(
        serde_crate["downloads"].as_u64().unwrap_or(0) > 0,
        "`serde` downloads field missing or zero: {serde_crate}"
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
#[ignore = "live: hits real docs.rs"]
async fn live_get_crate_docs_returns_markdown_for_serde_root() -> anyhow::Result<()> {
    let server = Server::new()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_crate_docs")
                .with_arguments(args(json!({ "crate_name": "serde" }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "live tool returned error: {result:?}"
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(parsed["crate_name"], "serde");
    // docs.rs always redirects `/latest/` to a concrete version — the
    // use case should have parsed it out of the final URL.
    assert!(
        parsed["resolved_version"].as_str().is_some(),
        "expected resolved_version in response: {parsed}",
    );
    let md = parsed["markdown"].as_str().expect("markdown string");
    // After <main> extraction, the markdown should be the crate-root
    // prose only — that always names the crate. If this stops being
    // true, the extraction heuristic has broken.
    assert!(
        md.to_lowercase().contains("serde"),
        "live markdown missing crate name: {}",
        &md.chars().take(500).collect::<String>(),
    );
    // Sidebar markers ("All Items", "Crate Items", search box widgets)
    // should be gone after extraction. Catching their reappearance
    // would tell us the upstream HTML shape drifted and we're shipping
    // full-page markdown again.
    assert!(
        !md.contains("Crate Items") && !md.contains("In crate "),
        "sidebar chrome leaked into extracted markdown: {}",
        &md.chars().take(500).collect::<String>(),
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
#[ignore = "live: hits real docs.rs"]
async fn live_search_crate_symbols_finds_serde_deserialize_trait() -> anyhow::Result<()> {
    let server = Server::new()?;
    let (client, server_handle) = spawn(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crate_symbols").with_arguments(args(json!({
                "crate_name": "serde",
                "query": "Deserialize",
                "kinds": ["trait"],
                "limit": 10,
            }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "live tool returned error: {result:?}"
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(parsed["crate_name"], "serde");
    assert!(
        parsed["resolved_version"].as_str().is_some(),
        "expected resolved_version in response: {parsed}",
    );
    // The Deserialize trait MUST appear when searching serde for
    // "Deserialize" + kind=trait. If this regresses, the parser is
    // missing a section or the URL convention has drifted.
    let items = parsed["items"].as_array().expect("items array");
    assert!(
        items.iter().any(|i| i["name"] == "Deserialize"),
        "Deserialize trait missing from serde symbol search: {parsed}",
    );
    // The returned path should be the one get_crate_docs accepts.
    let trait_item = items
        .iter()
        .find(|i| i["name"] == "Deserialize")
        .expect("Deserialize entry");
    let path = trait_item["path"].as_str().expect("path string");
    assert!(
        path.starts_with("trait.Deserialize.html"),
        "unexpected path for Deserialize trait: {path}",
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}

#[tokio::test]
#[ignore = "live: hits real crates.io API"]
async fn live_search_clamps_per_page_against_real_api() -> anyhow::Result<()> {
    let server = Server::new()?;
    let (client, server_handle) = spawn(server).await;

    // 200 is schema-valid (per_page is u8) but exceeds the use-case
    // ceiling of 100. This verifies the clamp fires against a real
    // upstream — crates.io would otherwise 400 on per_page > 100.
    let result = client
        .call_tool(
            CallToolRequestParams::new("search_crates")
                .with_arguments(args(json!({ "query": "tokio", "per_page": 200 }))),
        )
        .await?;

    assert!(
        !result.is_error.unwrap_or(false),
        "live tool returned error: {result:?}"
    );

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(
        parsed["per_page"].as_u64(),
        Some(100),
        "per_page should have been clamped to 100: {parsed}"
    );

    client.cancel().await?;
    let _ = server_handle.await;
    Ok(())
}
