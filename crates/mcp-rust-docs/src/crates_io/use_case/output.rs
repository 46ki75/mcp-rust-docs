/// Result returned by the use case, ready to be DTO-mapped by the
/// tool layer.
///
/// The `page` and `per_page` fields are the *effective* values used
/// against the registry — already defaulted and clamped — so the
/// caller can echo them back to the user without re-deriving.
#[derive(Debug, Clone)]
pub struct SearchCratesUseCaseOutput {
    /// Total number of matches across all pages.
    pub total: u64,
    /// 1-indexed page actually requested (after defaulting).
    pub page: u32,
    /// Page size actually requested (after clamping).
    pub per_page: u8,
    /// Crates on this page, in registry-returned order.
    pub crates: Vec<CrateSummary>,
}

/// A single crate hit with the version-selection policy already
/// applied — `version` here is the stable version when available,
/// falling back to `max_version` otherwise.
#[derive(Debug, Clone)]
pub struct CrateSummary {
    /// Crate name.
    pub name: String,
    /// Selected version: `max_stable_version` if present, else
    /// `max_version`. This is the policy choice the use case makes.
    pub version: String,
    /// Author-supplied short description from `Cargo.toml`.
    pub description: Option<String>,
    /// Lifetime download count.
    pub downloads: u64,
    /// Downloads in the last 90 days, when reported by the registry.
    pub recent_downloads: Option<u64>,
    /// Author-supplied documentation URL.
    pub documentation: Option<String>,
    /// Author-supplied homepage URL.
    pub homepage: Option<String>,
    /// Author-supplied source repository URL.
    pub repository: Option<String>,
    /// RFC 3339 timestamp of the most recent publish.
    pub updated_at: String,
}
