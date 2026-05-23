use std::sync::Arc;

use mcp_rust_docs::crates_io::repository::{
    CratesIoRepositoryError, CratesIoRepositoryStub, RepositoryCrateRecord,
    SearchCratesRepositoryOutput,
};
use mcp_rust_docs::crates_io::use_case::{
    CratesIoUseCase, CratesIoUseCaseError, SearchCratesUseCaseInput,
};

fn record() -> RepositoryCrateRecord {
    RepositoryCrateRecord {
        name: "tokio".into(),
        max_version: "1.40.0".into(),
        max_stable_version: Some("1.40.0".into()),
        description: Some("Async runtime".into()),
        downloads: 1,
        recent_downloads: Some(0),
        documentation: None,
        homepage: None,
        repository: None,
        updated_at: "2025-01-01T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn search_clamps_per_page_and_propagates_total() -> anyhow::Result<()> {
    let stub = Arc::new(CratesIoRepositoryStub::new());
    stub.enqueue(Ok(SearchCratesRepositoryOutput {
        total: 99,
        crates: vec![record()],
    }))
    .await;

    let use_case = CratesIoUseCase::new(stub);

    let out = use_case
        .search_crates(SearchCratesUseCaseInput {
            query: "tokio".into(),
            per_page: Some(250), // above max; should clamp to 100
            page: Some(0),       // below min; should clamp to 1
        })
        .await?;

    assert_eq!(out.total, 99);
    assert_eq!(out.per_page, 100);
    assert_eq!(out.page, 1);
    assert_eq!(out.crates.len(), 1);
    assert_eq!(out.crates[0].name, "tokio");
    assert_eq!(out.crates[0].version, "1.40.0");
    Ok(())
}

#[tokio::test]
async fn search_uses_defaults_when_not_specified() -> anyhow::Result<()> {
    let stub = Arc::new(CratesIoRepositoryStub::new());
    stub.enqueue(Ok(SearchCratesRepositoryOutput {
        total: 0,
        crates: vec![],
    }))
    .await;

    let use_case = CratesIoUseCase::new(stub);

    let out = use_case
        .search_crates(SearchCratesUseCaseInput {
            query: "anything".into(),
            per_page: None,
            page: None,
        })
        .await?;

    assert_eq!(out.page, 1);
    assert_eq!(out.per_page, 10);
    Ok(())
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let stub = Arc::new(CratesIoRepositoryStub::new());
    let use_case = CratesIoUseCase::new(stub);

    let err = use_case
        .search_crates(SearchCratesUseCaseInput {
            query: "   ".into(),
            per_page: None,
            page: None,
        })
        .await
        .expect_err("expected validation error");

    assert!(
        matches!(err, CratesIoUseCaseError::InvalidQuery(_)),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn search_bubbles_repository_failure() {
    let stub = Arc::new(CratesIoRepositoryStub::new());
    stub.enqueue(Err(CratesIoRepositoryError::UpstreamStatus {
        status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        body: "down for maintenance".into(),
    }))
    .await;

    let use_case = CratesIoUseCase::new(stub);

    let err = use_case
        .search_crates(SearchCratesUseCaseInput {
            query: "tokio".into(),
            per_page: None,
            page: None,
        })
        .await
        .expect_err("expected upstream failure");

    assert!(
        matches!(
            err,
            CratesIoUseCaseError::Repository(CratesIoRepositoryError::UpstreamStatus { .. })
        ),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn use_case_prefers_max_stable_when_available() -> anyhow::Result<()> {
    let stub = Arc::new(CratesIoRepositoryStub::new());
    let mut rec = record();
    rec.max_version = "2.0.0-beta".into();
    rec.max_stable_version = Some("1.9.0".into());
    stub.enqueue(Ok(SearchCratesRepositoryOutput {
        total: 1,
        crates: vec![rec],
    }))
    .await;

    let use_case = CratesIoUseCase::new(stub);

    let out = use_case
        .search_crates(SearchCratesUseCaseInput {
            query: "tokio".into(),
            per_page: None,
            page: None,
        })
        .await?;

    assert_eq!(out.crates[0].version, "1.9.0");
    Ok(())
}
