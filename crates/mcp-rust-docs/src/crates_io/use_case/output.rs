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

/// Per-crate metadata bundle returned by
/// [`CratesIoUseCase::get_crate_metadata`][crate::crates_io::use_case::CratesIoUseCase::get_crate_metadata].
///
/// Designed to answer the three "should I adopt this crate?" questions
/// (versions, features, dependencies) in one trip so the caller doesn't
/// have to thread together three separate API calls.
#[derive(Debug, Clone)]
pub struct CrateMetadata {
    /// Crate name as crates.io echoes it back.
    pub crate_name: String,
    /// The version the use case resolved against. If the input
    /// `version` was None or `latest`, this is the crate's stable
    /// (or fallback `max_version`) selection; otherwise it's the
    /// caller-specified concrete version.
    pub resolved_version: String,
    /// True when the resolved version is yanked on crates.io. Cached
    /// at the top level so the caller can flag this without scanning
    /// the `versions` list.
    pub resolved_version_yanked: bool,
    /// Recent versions of the crate, newest first, capped at
    /// [`VERSIONS_CAP`]. `versions_truncated` indicates whether more
    /// existed beyond the cap.
    pub versions: Vec<CrateVersion>,
    /// True when the registry reported more versions than
    /// [`VERSIONS_CAP`]. The caller can fall back to crates.io
    /// directly if it needs the full history.
    pub versions_truncated: bool,
    /// `Cargo.toml`'s `[features]` map for the resolved version. Each
    /// feature name maps to the list of features / optional
    /// dependencies it enables. `BTreeMap` gives stable ordering for
    /// snapshot/diff friendliness.
    pub features: std::collections::BTreeMap<String, Vec<String>>,
    /// Dependency breakdown for the resolved version. Counts cover
    /// the full population; named entries only cover runtime deps and
    /// are capped at [`RUNTIME_DEPS_CAP`].
    pub dependencies: DependencySummary,
}

/// A single published version's headline fields, as surfaced to
/// callers. The full `Cargo.toml` shape is too noisy for the kind of
/// "should I upgrade?" question this serves.
#[derive(Debug, Clone)]
pub struct CrateVersion {
    /// Semver string (`1.40.0`).
    pub num: String,
    /// True when this version has been yanked.
    pub yanked: bool,
    /// RFC 3339 timestamp of publication.
    pub created_at: String,
}

/// Aggregated view of a version's dependencies.
///
/// Counts are always exact across all kinds. The named `runtime` list
/// is capped at [`RUNTIME_DEPS_CAP`] — callers needing the full list
/// can fall back to the crates.io API directly.
#[derive(Debug, Clone)]
pub struct DependencySummary {
    /// Count of `[dependencies]` entries.
    pub runtime_count: usize,
    /// Count of `[dev-dependencies]` entries.
    pub dev_count: usize,
    /// Count of `[build-dependencies]` entries.
    pub build_count: usize,
    /// Count of deps declared with `optional = true`. Overlaps with
    /// the kind counts.
    pub optional_count: usize,
    /// Named runtime deps, capped at [`RUNTIME_DEPS_CAP`], in the
    /// registry-returned order. Dev/build deps intentionally omitted
    /// — they don't ship in the final binary, so they rarely drive
    /// adoption decisions.
    pub runtime: Vec<DependencyEntry>,
    /// True when the runtime list was truncated by the cap.
    pub runtime_truncated: bool,
}

/// A single named runtime dependency, kept minimal: agents
/// recommending crates need name + req + optional flag; transitive
/// graphs and target gating are out of scope for the summary view.
#[derive(Debug, Clone)]
pub struct DependencyEntry {
    /// Dependency crate name.
    pub name: String,
    /// Semver requirement as it appears in `Cargo.toml`.
    pub version_req: String,
    /// True when this dep is gated behind a feature.
    pub optional: bool,
}

/// Upper bound on the number of versions surfaced by the use case.
/// Picked so an agent reading metadata for a long-lived crate (tokio
/// has 200+ versions) doesn't get a wall of strings — but recent
/// history is still visible.
pub const VERSIONS_CAP: usize = 20;

/// Upper bound on the named runtime dependencies surfaced. The full
/// count is always reported in `DependencySummary.runtime_count` even
/// when the named list is truncated.
pub const RUNTIME_DEPS_CAP: usize = 15;
