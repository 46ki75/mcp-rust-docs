use serde::Serialize;

use crate::docs_rs::use_case::{
    DocHit, FetchCrateDocsUseCaseOutput, GrepCrateDocsUseCaseOutput,
    SearchCrateSymbolsUseCaseOutput, SymbolEntry,
};

/// JSON body returned by the `get_crate_docs` tool.
///
/// Mirrors [`FetchCrateDocsUseCaseOutput`] but with serde annotations
/// that drop missing optional fields rather than emitting `null`.
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
}

impl From<FetchCrateDocsUseCaseOutput> for GetCrateDocsResponse {
    fn from(output: FetchCrateDocsUseCaseOutput) -> Self {
        Self {
            crate_name: output.crate_name,
            resolved_version: output.resolved_version,
            url: output.final_url,
            markdown: output.markdown,
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

/// JSON body returned by the `grep_crate_docs` tool. Mirrors
/// [`SearchCrateSymbolsResponse`] shape with an extra `snippet` on
/// each hit.
#[derive(Debug, Serialize)]
pub struct GrepCrateDocsResponse {
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

impl From<GrepCrateDocsUseCaseOutput> for GrepCrateDocsResponse {
    fn from(output: GrepCrateDocsUseCaseOutput) -> Self {
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
