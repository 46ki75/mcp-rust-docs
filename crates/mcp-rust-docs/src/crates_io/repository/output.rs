/// Repository-tier projection of a crates.io search response.
///
/// Preserves the raw shape returned by the registry (both `max_version`
/// and `max_stable_version`, no field renaming) so the use case can
/// make the version-selection policy choice on its own.
#[derive(Debug, Clone)]
pub struct SearchCratesRepositoryOutput {
    /// Total number of matches across all pages, as reported by the
    /// registry's `meta.total` field.
    pub total: u64,
    /// Records for the current page, in the order returned by the API.
    pub crates: Vec<RepositoryCrateRecord>,
}

/// One crate hit, mirroring the fields the crates.io search endpoint
/// returns. Optional fields reflect what crate authors actually fill
/// in — most are missing on smaller crates.
#[derive(Debug, Clone)]
pub struct RepositoryCrateRecord {
    /// Crate name (the `id` on crates.io).
    pub name: String,
    /// Latest published version, including pre-releases.
    pub max_version: String,
    /// Latest non-prerelease version, if any. The use case prefers
    /// this over `max_version` when present.
    pub max_stable_version: Option<String>,
    /// Short description from the crate's `Cargo.toml`.
    pub description: Option<String>,
    /// Lifetime download count across all versions.
    pub downloads: u64,
    /// Downloads in the last 90 days, when the registry reports it.
    pub recent_downloads: Option<u64>,
    /// Author-supplied documentation URL (usually docs.rs, sometimes
    /// a custom site).
    pub documentation: Option<String>,
    /// Author-supplied homepage URL.
    pub homepage: Option<String>,
    /// Author-supplied source repository URL.
    pub repository: Option<String>,
    /// RFC 3339 timestamp of the most recent publish for the crate.
    pub updated_at: String,
}
