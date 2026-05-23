/// Pre-validation arguments accepted by the use case.
///
/// The use case is what enforces "non-empty query, clamp `per_page`
/// to 1-100, default page to 1" — so unlike
/// [`SearchCratesRepositoryInput`][crate::crates_io::repository::SearchCratesRepositoryInput],
/// the optional fields here have not been resolved yet.
#[derive(Debug, Clone)]
pub struct SearchCratesUseCaseInput {
    /// Search query. Whitespace-only inputs are rejected as
    /// `InvalidQuery` by the use case.
    pub query: String,
    /// Optional page size. `None` defaults to 10; values outside
    /// `1..=100` are clamped, not rejected.
    pub per_page: Option<u8>,
    /// Optional 1-indexed page. `None` and `0` both default to 1.
    pub page: Option<u32>,
}

/// Pre-validation arguments for [`CratesIoUseCase::get_crate_metadata`].
///
/// `version` accepts a concrete semver string (`1.40.0`) or the literal
/// `latest` (or `None`); semver ranges are not supported because
/// crates.io's per-version endpoint requires a concrete identifier
/// and the use case won't synthesize one from a range.
///
/// [`CratesIoUseCase::get_crate_metadata`]:
/// crate::crates_io::use_case::CratesIoUseCase::get_crate_metadata
#[derive(Debug, Clone)]
pub struct GetCrateMetadataUseCaseInput {
    /// Crate name as published on crates.io. Whitespace-trimmed by
    /// the use case; empty after trim is `InvalidQuery`.
    pub crate_name: String,
    /// Optional version. `None` or `Some("latest")` resolves to the
    /// crate's `max_stable_version` (falling back to `max_version`).
    pub version: Option<String>,
}
