/// Use case error type.
pub mod error;
/// Use case input types.
pub mod input;
/// Use case output types.
pub mod output;

use std::sync::Arc;

pub use self::error::CratesIoUseCaseError;
pub use self::input::{GetCrateMetadataUseCaseInput, SearchCratesUseCaseInput};
pub use self::output::{
    CrateMetadata, CrateSummary, CrateVersion, DependencyEntry, DependencySummary,
    RUNTIME_DEPS_CAP, SearchCratesUseCaseOutput, VERSIONS_CAP,
};

use crate::crates_io::repository::{
    CratesIoRepository, FetchCrateInput, FetchCrateRepositoryOutput,
    FetchCrateVersionDependenciesInput, RepositoryCrateRecord, RepositoryDependency,
    RepositoryDependencyKind, SearchCratesRepositoryInput, SearchCratesRepositoryOutput,
};

const DEFAULT_PER_PAGE: u8 = 10;
const MAX_PER_PAGE: u8 = 100;
const DEFAULT_PAGE: u32 = 1;
const LATEST_VERSION: &str = "latest";

/// Use case for searching crates on crates.io.
///
/// Holds the repository behind `Arc<dyn>` so production wiring and
/// stub-backed unit tests share the same code path. All input
/// validation and policy (defaults, clamping, stable-version
/// preference) lives here — the repository is dumb I/O and the tool
/// layer is dumb DTO translation.
pub struct CratesIoUseCase {
    repository: Arc<dyn CratesIoRepository>,
}

impl CratesIoUseCase {
    /// Build a use case backed by the given repository.
    pub fn new(repository: Arc<dyn CratesIoRepository>) -> Self {
        Self { repository }
    }

    /// Validate, default-and-clamp the input, then issue a search
    /// against the underlying repository.
    ///
    /// Whitespace-only queries are rejected as
    /// [`CratesIoUseCaseError::InvalidQuery`]; otherwise repository
    /// failures bubble through [`CratesIoUseCaseError::Repository`].
    #[tracing::instrument(skip(self))]
    pub async fn search_crates(
        &self,
        input: SearchCratesUseCaseInput,
    ) -> Result<SearchCratesUseCaseOutput, CratesIoUseCaseError> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(CratesIoUseCaseError::InvalidQuery(
                "query must not be empty".into(),
            ));
        }

        let per_page = input
            .per_page
            .unwrap_or(DEFAULT_PER_PAGE)
            .clamp(1, MAX_PER_PAGE);
        let page = input.page.unwrap_or(DEFAULT_PAGE).max(1);

        let repo_output = self
            .repository
            .search_crates(SearchCratesRepositoryInput {
                query: query.to_string(),
                per_page,
                page,
            })
            .await?;

        Ok(into_use_case_output(repo_output, page, per_page))
    }

    /// Fetch the per-crate metadata bundle: recent versions, the
    /// resolved version's features, and a dependency summary.
    ///
    /// Two upstream calls are made in sequence: the per-crate
    /// aggregate (gives the version list + features) and the
    /// per-version dependencies. The second depends on the resolved
    /// version, so they can't be parallelised.
    ///
    /// Version resolution policy:
    /// - `None` or `Some("latest")` → `max_stable_version` if present,
    ///   else `max_version`.
    /// - Some concrete semver string → must appear in the versions
    ///   list; otherwise `InvalidQuery` (typo or unpublished).
    /// - Semver ranges (`^1.0`) are not supported and surface as
    ///   `InvalidQuery`.
    #[tracing::instrument(skip(self))]
    pub async fn get_crate_metadata(
        &self,
        input: GetCrateMetadataUseCaseInput,
    ) -> Result<CrateMetadata, CratesIoUseCaseError> {
        let crate_name = input.crate_name.trim();
        if crate_name.is_empty() {
            return Err(CratesIoUseCaseError::InvalidQuery(
                "crate_name must not be empty".into(),
            ));
        }
        let crate_name = crate_name.to_string();

        let aggregate = self
            .repository
            .fetch_crate(FetchCrateInput {
                crate_name: crate_name.clone(),
            })
            .await?;

        let resolved_version = resolve_metadata_version(&aggregate, input.version.as_deref())?;

        // Find the resolved version entry so we can read its features
        // and yanked status. Guaranteed Some because
        // `resolve_metadata_version` only returns a string that came
        // from this same versions list.
        let resolved_entry = aggregate
            .versions
            .iter()
            .find(|v| v.num == resolved_version)
            .expect("resolved version is sourced from aggregate.versions");
        let features = resolved_entry.features.clone();
        let resolved_yanked = resolved_entry.yanked;

        let versions_total = aggregate.versions.len();
        let versions: Vec<CrateVersion> = aggregate
            .versions
            .iter()
            .take(VERSIONS_CAP)
            .map(|v| CrateVersion {
                num: v.num.clone(),
                yanked: v.yanked,
                created_at: v.created_at.clone(),
            })
            .collect();
        let versions_truncated = versions_total > VERSIONS_CAP;

        let deps_output = self
            .repository
            .fetch_crate_version_dependencies(FetchCrateVersionDependenciesInput {
                crate_name: crate_name.clone(),
                version: resolved_version.clone(),
            })
            .await?;
        let dependencies = summarize_dependencies(deps_output.dependencies);

        Ok(CrateMetadata {
            crate_name: aggregate.name,
            resolved_version,
            resolved_version_yanked: resolved_yanked,
            versions,
            versions_truncated,
            features,
            dependencies,
        })
    }
}

/// Apply the version-selection policy. Lives outside the impl for
/// unit-testability without a use case fixture.
fn resolve_metadata_version(
    aggregate: &FetchCrateRepositoryOutput,
    requested: Option<&str>,
) -> Result<String, CratesIoUseCaseError> {
    let requested = requested.map(str::trim).filter(|s| !s.is_empty());

    let pick_latest = match requested {
        None => true,
        Some(v) => v.eq_ignore_ascii_case(LATEST_VERSION),
    };
    if pick_latest {
        return Ok(aggregate
            .max_stable_version
            .clone()
            .unwrap_or_else(|| aggregate.max_version.clone()));
    }

    let asked = requested.expect("pick_latest=false implies Some");
    if aggregate.versions.iter().any(|v| v.num == asked) {
        Ok(asked.to_string())
    } else {
        Err(CratesIoUseCaseError::InvalidQuery(format!(
            "version `{}` not found for crate `{}` (semver ranges not \
             supported here — pass a concrete version like `1.40.0` or `latest`)",
            asked, aggregate.name,
        )))
    }
}

/// Project the repository dependency list into the use-case summary
/// shape: per-kind counts (full) plus a capped named list of the
/// runtime deps.
fn summarize_dependencies(deps: Vec<RepositoryDependency>) -> DependencySummary {
    let mut runtime_count = 0usize;
    let mut dev_count = 0usize;
    let mut build_count = 0usize;
    let mut optional_count = 0usize;
    let mut runtime = Vec::new();

    for dep in deps {
        if dep.optional {
            optional_count += 1;
        }
        match dep.kind {
            RepositoryDependencyKind::Normal => {
                runtime_count += 1;
                if runtime.len() < RUNTIME_DEPS_CAP {
                    runtime.push(DependencyEntry {
                        name: dep.name,
                        version_req: dep.req,
                        optional: dep.optional,
                    });
                }
            }
            RepositoryDependencyKind::Dev => dev_count += 1,
            RepositoryDependencyKind::Build => build_count += 1,
        }
    }

    let runtime_truncated = runtime_count > runtime.len();

    DependencySummary {
        runtime_count,
        dev_count,
        build_count,
        optional_count,
        runtime,
        runtime_truncated,
    }
}

fn into_use_case_output(
    output: SearchCratesRepositoryOutput,
    page: u32,
    per_page: u8,
) -> SearchCratesUseCaseOutput {
    SearchCratesUseCaseOutput {
        total: output.total,
        page,
        per_page,
        crates: output.crates.into_iter().map(into_summary).collect(),
    }
}

fn into_summary(record: RepositoryCrateRecord) -> CrateSummary {
    CrateSummary {
        version: record.max_stable_version.unwrap_or(record.max_version),
        name: record.name,
        description: record.description,
        downloads: record.downloads,
        recent_downloads: record.recent_downloads,
        documentation: record.documentation,
        homepage: record.homepage,
        repository: record.repository,
        updated_at: record.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crates_io::repository::CratesIoRepositoryStub;

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
        use crate::crates_io::repository::CratesIoRepositoryError;

        let stub = Arc::new(CratesIoRepositoryStub::new());
        stub.enqueue(Err(CratesIoRepositoryError::UpstreamStatus {
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
            url: "https://crates.io/api/v1/crates".into(),
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

    // ---- get_crate_metadata --------------------------------------------------

    use crate::crates_io::repository::{
        CratesIoRepositoryError, FetchCrateRepositoryOutput,
        FetchCrateVersionDependenciesRepositoryOutput, RepositoryCrateVersion,
        RepositoryDependency, RepositoryDependencyKind,
    };
    use std::collections::BTreeMap;

    fn version_entry(num: &str, yanked: bool) -> RepositoryCrateVersion {
        let mut features = BTreeMap::new();
        features.insert("default".into(), vec!["std".into()]);
        features.insert("derive".into(), vec!["serde_derive".into()]);
        RepositoryCrateVersion {
            num: num.into(),
            yanked,
            created_at: "2025-01-01T00:00:00Z".into(),
            features,
        }
    }

    fn aggregate_for(
        name: &str,
        versions: Vec<RepositoryCrateVersion>,
    ) -> FetchCrateRepositoryOutput {
        let max_version = versions
            .first()
            .map(|v| v.num.clone())
            .unwrap_or_else(|| "0.0.0".into());
        FetchCrateRepositoryOutput {
            name: name.into(),
            max_version: max_version.clone(),
            max_stable_version: Some(max_version),
            versions,
        }
    }

    fn dep(name: &str, kind: RepositoryDependencyKind, optional: bool) -> RepositoryDependency {
        RepositoryDependency {
            name: name.into(),
            req: "^1.0".into(),
            kind,
            optional,
        }
    }

    #[tokio::test]
    async fn metadata_resolves_latest_to_max_stable_version() -> anyhow::Result<()> {
        // `max_stable_version` (1.40.0) must beat `max_version`
        // (2.0.0-beta) when the caller didn't pin a concrete version.
        let stub = Arc::new(CratesIoRepositoryStub::new());
        let mut aggregate = aggregate_for(
            "serde",
            vec![
                version_entry("2.0.0-beta", false),
                version_entry("1.40.0", false),
            ],
        );
        aggregate.max_version = "2.0.0-beta".into();
        aggregate.max_stable_version = Some("1.40.0".into());
        stub.enqueue_crate(Ok(aggregate)).await;
        stub.enqueue_dependencies(Ok(FetchCrateVersionDependenciesRepositoryOutput {
            dependencies: vec![],
        }))
        .await;

        let use_case = CratesIoUseCase::new(stub);
        let metadata = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "serde".into(),
                version: None,
            })
            .await?;

        assert_eq!(metadata.resolved_version, "1.40.0");
        assert!(!metadata.resolved_version_yanked);
        Ok(())
    }

    #[tokio::test]
    async fn metadata_resolves_latest_to_max_version_when_no_stable() -> anyhow::Result<()> {
        // Pre-1.0 crates often only have prerelease/non-stable versions.
        // `max_stable_version` is None — must fall back to `max_version`.
        let stub = Arc::new(CratesIoRepositoryStub::new());
        let mut aggregate = aggregate_for("brand-new", vec![version_entry("0.0.1-alpha.1", false)]);
        aggregate.max_stable_version = None;
        aggregate.max_version = "0.0.1-alpha.1".into();
        stub.enqueue_crate(Ok(aggregate)).await;
        stub.enqueue_dependencies(Ok(FetchCrateVersionDependenciesRepositoryOutput {
            dependencies: vec![],
        }))
        .await;

        let use_case = CratesIoUseCase::new(stub);
        let metadata = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "brand-new".into(),
                version: Some("latest".into()),
            })
            .await?;

        assert_eq!(metadata.resolved_version, "0.0.1-alpha.1");
        Ok(())
    }

    #[tokio::test]
    async fn metadata_rejects_concrete_version_not_in_list() {
        // The crate exists but the requested version doesn't — surface
        // as InvalidQuery so the tool layer renders it as caller-fixable
        // ("Invalid request: ..."), not "Upstream failure".
        let stub = Arc::new(CratesIoRepositoryStub::new());
        stub.enqueue_crate(Ok(aggregate_for(
            "anyhow",
            vec![version_entry("1.0.86", false)],
        )))
        .await;

        let use_case = CratesIoUseCase::new(stub);
        let err = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "anyhow".into(),
                version: Some("9.9.9".into()),
            })
            .await
            .expect_err("expected InvalidQuery for unknown version");
        assert!(
            matches!(err, CratesIoUseCaseError::InvalidQuery(ref msg) if msg.contains("9.9.9")),
            "unexpected error: {err:?}",
        );
    }

    #[tokio::test]
    async fn metadata_caps_versions_at_limit_and_flags_truncation() -> anyhow::Result<()> {
        // 25 versions enqueued, cap is 20 — must surface first 20 +
        // versions_truncated=true.
        let stub = Arc::new(CratesIoRepositoryStub::new());
        let versions: Vec<_> = (0..25)
            .map(|i| version_entry(&format!("1.{i}.0"), false))
            .collect();
        stub.enqueue_crate(Ok(aggregate_for("big", versions))).await;
        stub.enqueue_dependencies(Ok(FetchCrateVersionDependenciesRepositoryOutput {
            dependencies: vec![],
        }))
        .await;

        let use_case = CratesIoUseCase::new(stub);
        let metadata = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "big".into(),
                version: None,
            })
            .await?;

        assert_eq!(metadata.versions.len(), VERSIONS_CAP);
        assert!(metadata.versions_truncated);
        Ok(())
    }

    #[tokio::test]
    async fn metadata_summarizes_deps_by_kind_with_full_counts() -> anyhow::Result<()> {
        // 20 runtime, 3 dev, 2 build, 4 optional (overlapping with
        // runtime). Counts must be full; named runtime list capped
        // at 15.
        let stub = Arc::new(CratesIoRepositoryStub::new());
        stub.enqueue_crate(Ok(aggregate_for(
            "heavy",
            vec![version_entry("1.0.0", false)],
        )))
        .await;

        let mut deps = Vec::new();
        for i in 0..20 {
            deps.push(dep(
                &format!("runtime-{i}"),
                RepositoryDependencyKind::Normal,
                i < 4, // first 4 are optional
            ));
        }
        for i in 0..3 {
            deps.push(dep(
                &format!("dev-{i}"),
                RepositoryDependencyKind::Dev,
                false,
            ));
        }
        for i in 0..2 {
            deps.push(dep(
                &format!("build-{i}"),
                RepositoryDependencyKind::Build,
                false,
            ));
        }
        stub.enqueue_dependencies(Ok(FetchCrateVersionDependenciesRepositoryOutput {
            dependencies: deps,
        }))
        .await;

        let use_case = CratesIoUseCase::new(stub);
        let metadata = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "heavy".into(),
                version: Some("1.0.0".into()),
            })
            .await?;

        let summary = &metadata.dependencies;
        assert_eq!(summary.runtime_count, 20);
        assert_eq!(summary.dev_count, 3);
        assert_eq!(summary.build_count, 2);
        assert_eq!(summary.optional_count, 4);
        assert_eq!(summary.runtime.len(), RUNTIME_DEPS_CAP);
        assert!(summary.runtime_truncated);
        Ok(())
    }

    #[tokio::test]
    async fn metadata_propagates_repository_not_found_on_unknown_crate() {
        // No crate enqueued → stub's default 404 fires. Use case must
        // propagate as Repository(NotFound), not InvalidQuery — the
        // crate-name shape was valid, it just isn't on crates.io.
        let stub = Arc::new(CratesIoRepositoryStub::new());
        let use_case = CratesIoUseCase::new(stub);
        let err = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "definitely-not-real".into(),
                version: None,
            })
            .await
            .expect_err("expected NotFound from stub default");
        assert!(
            matches!(
                err,
                CratesIoUseCaseError::Repository(CratesIoRepositoryError::NotFound { .. })
            ),
            "unexpected error: {err:?}",
        );
    }

    #[tokio::test]
    async fn metadata_rejects_empty_crate_name() {
        let stub = Arc::new(CratesIoRepositoryStub::new());
        let use_case = CratesIoUseCase::new(stub);
        let err = use_case
            .get_crate_metadata(GetCrateMetadataUseCaseInput {
                crate_name: "   ".into(),
                version: None,
            })
            .await
            .expect_err("expected InvalidQuery for blank crate name");
        assert!(matches!(err, CratesIoUseCaseError::InvalidQuery(_)));
    }
}
