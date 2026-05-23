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
