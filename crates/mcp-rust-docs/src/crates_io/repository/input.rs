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

/// Arguments for the per-crate metadata fetch. The use case has
/// already trimmed the crate name before constructing this — the
/// repository assembles the URL verbatim.
#[derive(Debug, Clone)]
pub struct FetchCrateInput {
    /// Crate name as it appears on crates.io (`tokio`, `tokio-util`).
    pub crate_name: String,
}

/// Arguments for the dependencies-of-a-version fetch. Both fields are
/// already resolved — `version` is concrete (not `latest`), and the
/// crate name is trimmed.
#[derive(Debug, Clone)]
pub struct FetchCrateVersionDependenciesInput {
    /// Crate name.
    pub crate_name: String,
    /// Concrete semver version string. The repository does not accept
    /// `latest` here — version resolution is the use case's job.
    pub version: String,
}
