/// Arguments sent down to the repository layer. All fields are
/// post-validation: the use case has already clamped `per_page` to a
/// safe range and trimmed `query` to a non-empty string before this
/// type is constructed.
#[derive(Debug, Clone)]
pub struct SearchCratesRepositoryInput {
    /// Search string, passed as `?q=` to the crates.io API.
    pub query: String,
    /// Page size already clamped to the registry's accepted range
    /// (1-100). `u8` makes the upper bound part of the type.
    pub per_page: u8,
    /// 1-indexed page number.
    pub page: u32,
}
