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

/// Repository-tier projection of `GET /api/v1/crates/{name}`. Carries
/// the per-crate aggregate fields plus the full version list so the
/// use case can both pick the canonical version and cap/order the
/// list it surfaces to callers.
#[derive(Debug, Clone)]
pub struct FetchCrateRepositoryOutput {
    /// Crate name as the registry echoes it back.
    pub name: String,
    /// Latest published version including pre-releases.
    pub max_version: String,
    /// Latest non-prerelease version when one exists.
    pub max_stable_version: Option<String>,
    /// Every published version the registry knows about, in the order
    /// it returned them (typically newest first, but the use case
    /// does not rely on that — it caps after applying its own policy).
    pub versions: Vec<RepositoryCrateVersion>,
}

/// One published version, with the per-version fields the use case
/// needs. `features` is the raw map from `Cargo.toml`'s `[features]`
/// section: each feature name mapped to the list of features /
/// optional dependencies it enables.
#[derive(Debug, Clone)]
pub struct RepositoryCrateVersion {
    /// Semver string (`1.40.0`, `1.0.0-beta.3`).
    pub num: String,
    /// True when the registry has yanked this version. Yanked
    /// versions still resolve in `Cargo.lock`s that already pin
    /// them, but new resolution should avoid them.
    pub yanked: bool,
    /// RFC 3339 timestamp of when this version was published.
    pub created_at: String,
    /// Feature map from `Cargo.toml`'s `[features]` section.
    pub features: std::collections::BTreeMap<String, Vec<String>>,
}

/// Repository-tier projection of `GET /api/v1/crates/{name}/{version}/dependencies`.
#[derive(Debug, Clone)]
pub struct FetchCrateVersionDependenciesRepositoryOutput {
    /// Every declared dependency for the requested version.
    pub dependencies: Vec<RepositoryDependency>,
}

/// A single dependency record. `kind` is the `[dependencies]` /
/// `[dev-dependencies]` / `[build-dependencies]` section the dep was
/// declared in.
#[derive(Debug, Clone)]
pub struct RepositoryDependency {
    /// Dependency crate name (mapped from crates.io's `crate_id`
    /// field, which despite the name is the dep's crate name string).
    pub name: String,
    /// Semver requirement string as it appears in `Cargo.toml`
    /// (`^1.0`, `>=2.0, <3.0`, `=0.1.5`).
    pub req: String,
    /// Which `Cargo.toml` section the dep was declared in.
    pub kind: RepositoryDependencyKind,
    /// True for `optional = true` deps.
    pub optional: bool,
}

/// Which dependency section a dep was declared in. Mirrors
/// crates.io's `kind` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryDependencyKind {
    /// Declared under `[dependencies]`.
    Normal,
    /// Declared under `[dev-dependencies]`.
    Dev,
    /// Declared under `[build-dependencies]`.
    Build,
}
