use std::collections::BTreeMap;

use serde::Serialize;

use crate::crates_io::use_case::{CrateMetadata, CrateVersion, DependencyEntry, DependencySummary};
use crate::docs_rs::use_case::{
    DocHit, FetchCrateDocsUseCaseOutput, SearchCrateDocsUseCaseOutput,
    SearchCrateSymbolsUseCaseOutput, SymbolEntry,
};

/// JSON body returned by the `get_crate_docs` tool.
///
/// Mirrors [`FetchCrateDocsUseCaseOutput`] but with serde annotations
/// that drop missing optional fields rather than emitting `null`. On
/// crate-root calls (no `path`), an optional `metadata` block is
/// attached carrying versions / features / dependencies fetched from
/// crates.io in parallel with the docs page. Drill-down calls keep
/// the lean response.
#[derive(Debug, Serialize)]
pub struct GetCrateDocsResponse {
    /// Crate name as requested.
    pub crate_name: String,

    /// Concrete version docs.rs actually served, parsed out of the
    /// redirected URL. Omitted from the JSON when the URL shape didn't
    /// match the expected `/{crate}/{version}/...` layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_version: Option<String>,

    /// Canonical URL of the page on docs.rs, after redirects.
    pub url: String,

    /// Page contents converted to Markdown. The use case extracts
    /// the `<main>` element from the rustdoc HTML before conversion,
    /// so the sidebar / search box / footer are not included.
    pub markdown: String,

    /// Per-crate metadata (versions, features, dependencies) when
    /// the call targets the crate root. Omitted on drill-down calls
    /// (when `path` was supplied) and when the metadata fetch
    /// fails — in the latter case `metadata_error` carries the
    /// reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataDto>,

    /// Set when a crate-root call attempted to fetch metadata but
    /// the crates.io call failed. The docs page still ships
    /// (metadata is best-effort), and this field surfaces the
    /// underlying error so the caller can decide whether to retry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_error: Option<String>,
}

impl GetCrateDocsResponse {
    /// Build a response from the docs.rs use case output alone — used
    /// for drill-down calls where no metadata was fetched.
    pub fn from_docs_only(output: FetchCrateDocsUseCaseOutput) -> Self {
        Self {
            crate_name: output.crate_name,
            resolved_version: output.resolved_version,
            url: output.final_url,
            markdown: output.markdown,
            metadata: None,
            metadata_error: None,
        }
    }

    /// Build a response from the docs.rs output plus the result of a
    /// metadata fetch attempt. `Ok` populates `metadata`; `Err`
    /// populates `metadata_error` with the formatted error message so
    /// the docs still ship.
    pub fn from_docs_and_metadata(
        output: FetchCrateDocsUseCaseOutput,
        metadata: Result<CrateMetadata, String>,
    ) -> Self {
        let mut response = Self::from_docs_only(output);
        match metadata {
            Ok(metadata) => response.metadata = Some(MetadataDto::from(metadata)),
            Err(message) => response.metadata_error = Some(message),
        }
        response
    }
}

/// Wire shape of the per-crate metadata bundle.
#[derive(Debug, Serialize)]
pub struct MetadataDto {
    /// Crate name as crates.io echoes it back.
    pub crate_name: String,
    /// Version the use case resolved for features and dependencies.
    pub resolved_version: String,
    /// True when `resolved_version` is yanked.
    pub resolved_version_yanked: bool,
    /// Recent versions (newest first, capped).
    pub versions: Vec<CrateVersionDto>,
    /// True when more versions existed beyond the cap.
    pub versions_truncated: bool,
    /// `Cargo.toml`'s `[features]` map for the resolved version.
    pub features: BTreeMap<String, Vec<String>>,
    /// Dependency breakdown for the resolved version.
    pub dependencies: DependencySummaryDto,
}

/// Wire shape of a single version entry.
#[derive(Debug, Serialize)]
pub struct CrateVersionDto {
    /// Semver string.
    pub num: String,
    /// Yanked flag.
    pub yanked: bool,
    /// RFC 3339 publish timestamp.
    pub created_at: String,
}

/// Wire shape of the dependency summary.
#[derive(Debug, Serialize)]
pub struct DependencySummaryDto {
    /// `[dependencies]` count.
    pub runtime_count: usize,
    /// `[dev-dependencies]` count.
    pub dev_count: usize,
    /// `[build-dependencies]` count.
    pub build_count: usize,
    /// Count of optional deps (overlaps with kind counts).
    pub optional_count: usize,
    /// Named runtime deps (capped).
    pub runtime: Vec<DependencyEntryDto>,
    /// True when the runtime list was truncated.
    pub runtime_truncated: bool,
}

/// Wire shape of one runtime dependency entry.
#[derive(Debug, Serialize)]
pub struct DependencyEntryDto {
    /// Dependency crate name.
    pub name: String,
    /// Semver requirement string.
    pub version_req: String,
    /// True for optional deps.
    pub optional: bool,
}

impl From<CrateMetadata> for MetadataDto {
    fn from(value: CrateMetadata) -> Self {
        Self {
            crate_name: value.crate_name,
            resolved_version: value.resolved_version,
            resolved_version_yanked: value.resolved_version_yanked,
            versions: value
                .versions
                .into_iter()
                .map(CrateVersionDto::from)
                .collect(),
            versions_truncated: value.versions_truncated,
            features: value.features,
            dependencies: DependencySummaryDto::from(value.dependencies),
        }
    }
}

impl From<CrateVersion> for CrateVersionDto {
    fn from(value: CrateVersion) -> Self {
        Self {
            num: value.num,
            yanked: value.yanked,
            created_at: value.created_at,
        }
    }
}

impl From<DependencySummary> for DependencySummaryDto {
    fn from(value: DependencySummary) -> Self {
        Self {
            runtime_count: value.runtime_count,
            dev_count: value.dev_count,
            build_count: value.build_count,
            optional_count: value.optional_count,
            runtime: value
                .runtime
                .into_iter()
                .map(DependencyEntryDto::from)
                .collect(),
            runtime_truncated: value.runtime_truncated,
        }
    }
}

impl From<DependencyEntry> for DependencyEntryDto {
    fn from(value: DependencyEntry) -> Self {
        Self {
            name: value.name,
            version_req: value.version_req,
            optional: value.optional,
        }
    }
}

/// JSON body returned by the `search_crate_symbols` tool.
#[derive(Debug, Serialize)]
pub struct SearchCrateSymbolsResponse {
    /// Crate name as requested.
    pub crate_name: String,

    /// Concrete version docs.rs served the index from, parsed out
    /// of the redirected URL. Omitted from the JSON when the URL
    /// shape didn't match `/{crate}/{version}/...`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_version: Option<String>,

    /// Total items matching the filters before truncation. If this
    /// exceeds `items.len()`, the caller should narrow `query` or
    /// raise `limit`.
    pub total_matched: usize,

    /// `true` when more items matched than were returned.
    pub truncated: bool,

    /// Matched items.
    pub items: Vec<SymbolDto>,
}

/// Wire representation of a single matched symbol.
#[derive(Debug, Serialize)]
pub struct SymbolDto {
    /// Normalised rustdoc kind (`struct`, `enum`, `trait`, `fn`, …).
    pub kind: String,
    /// Fully-qualified item name as rustdoc renders it.
    pub name: String,
    /// URL-path tail relative to the crate docs root. Pass this
    /// verbatim to `get_crate_docs.path` to fetch the item's docs.
    pub path: String,
}

impl From<SearchCrateSymbolsUseCaseOutput> for SearchCrateSymbolsResponse {
    fn from(output: SearchCrateSymbolsUseCaseOutput) -> Self {
        Self {
            crate_name: output.crate_name,
            resolved_version: output.resolved_version,
            total_matched: output.total_matched,
            truncated: output.truncated,
            items: output.items.into_iter().map(SymbolDto::from).collect(),
        }
    }
}

impl From<SymbolEntry> for SymbolDto {
    fn from(entry: SymbolEntry) -> Self {
        Self {
            kind: entry.kind,
            name: entry.name,
            path: entry.path,
        }
    }
}

/// JSON body returned by the `search_crate_docs` tool. Mirrors
/// [`SearchCrateSymbolsResponse`] shape with an extra `snippet` on
/// each hit.
#[derive(Debug, Serialize)]
pub struct SearchCrateDocsResponse {
    /// Crate name as requested.
    pub crate_name: String,

    /// Concrete version docs.rs served the JSON from. Omitted from
    /// JSON when the URL shape didn't match `/crate/{name}/{version}/...`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_version: Option<String>,

    /// Total items matching the filters before truncation.
    pub total_matched: usize,

    /// `true` when more items matched than were returned.
    pub truncated: bool,

    /// Matched items, ranked by name-match bonus, hit count, then
    /// qualified name.
    pub items: Vec<DocHitDto>,
}

/// Wire representation of a single doc-comment match.
#[derive(Debug, Serialize)]
pub struct DocHitDto {
    /// Normalised rustdoc kind.
    pub kind: String,
    /// Fully-qualified item name as rustdoc renders it.
    pub name: String,
    /// URL-path tail under the crate's docs root. Pass to
    /// `get_crate_docs.path` to read the full docs.
    pub path: String,
    /// Short excerpt centered on the first match, with `…` markers
    /// when truncated.
    pub snippet: String,
}

impl From<SearchCrateDocsUseCaseOutput> for SearchCrateDocsResponse {
    fn from(output: SearchCrateDocsUseCaseOutput) -> Self {
        Self {
            crate_name: output.crate_name,
            resolved_version: output.resolved_version,
            total_matched: output.total_matched,
            truncated: output.truncated,
            items: output.items.into_iter().map(DocHitDto::from).collect(),
        }
    }
}

impl From<DocHit> for DocHitDto {
    fn from(hit: DocHit) -> Self {
        Self {
            kind: hit.kind,
            name: hit.name,
            path: hit.path,
            snippet: hit.snippet,
        }
    }
}
