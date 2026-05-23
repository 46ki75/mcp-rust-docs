/// Result returned by the use case, ready to be DTO-mapped by the
/// tool layer.
///
/// `resolved_version` reflects what docs.rs actually served — when
/// the caller asks for `latest`, this is the concrete version after
/// redirect. `final_url` is the canonical URL the doc page lives at;
/// useful to echo back so the user can click through.
#[derive(Debug, Clone)]
pub struct FetchCrateDocsUseCaseOutput {
    /// Crate name as requested (post-trim).
    pub crate_name: String,
    /// Version actually served by docs.rs, parsed out of the final
    /// URL. `None` if the URL didn't follow the expected
    /// `/{crate}/{version}/...` shape.
    pub resolved_version: Option<String>,
    /// Final URL the docs page was served from, after any redirects.
    pub final_url: String,
    /// Page contents converted to Markdown.
    pub markdown: String,
}

/// Result returned by the symbol-search use case.
///
/// `total_matched` is the number of items that satisfied the filters
/// *before* truncation; compare against `items.len()` (or check
/// `truncated`) to know whether the caller needs to narrow the query.
#[derive(Debug, Clone)]
pub struct SearchCrateSymbolsUseCaseOutput {
    /// Crate name as requested (post-normalisation).
    pub crate_name: String,
    /// Concrete version docs.rs served the `all.html` from, parsed
    /// out of the final URL. `None` if the URL shape was unexpected.
    pub resolved_version: Option<String>,
    /// Total items matching `query`/`kinds` before `limit` was
    /// applied.
    pub total_matched: usize,
    /// `true` when more items matched than the limit returned.
    pub truncated: bool,
    /// Matched items in the order they appeared in `all.html`
    /// (alphabetical within each kind, grouped by kind in rustdoc's
    /// preferred order: structs, enums, traits, …).
    pub items: Vec<SymbolEntry>,
}

/// One item from a crate's `all.html` page.
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    /// Normalised rustdoc kind: `struct`, `enum`, `trait`, `fn`,
    /// `macro`, `derive`, `attribute`, `type`, `module`, `constant`,
    /// `static`, `union`, `primitive`.
    pub kind: String,
    /// Fully-qualified item name as rustdoc renders it
    /// (e.g. `de::value::U8Deserializer`).
    pub name: String,
    /// URL-path tail relative to the crate's docs root
    /// (e.g. `de/value/struct.U8Deserializer.html`). Use this
    /// verbatim as the `path` argument to `get_crate_docs`.
    pub path: String,
}

/// Result returned by the doc-comment search use case.
///
/// Same shape as [`SearchCrateSymbolsUseCaseOutput`] so models that
/// know one tool's response can read the other's without re-learning.
/// `total_matched` is the pre-truncation count so the caller can tell
/// when to narrow the query.
#[derive(Debug, Clone)]
pub struct SearchCrateDocsUseCaseOutput {
    /// Crate name as requested (post-normalisation).
    pub crate_name: String,
    /// Concrete version docs.rs served the JSON from, parsed out of
    /// the redirected URL. `None` if the URL shape was unexpected.
    pub resolved_version: Option<String>,
    /// Total items whose doc comments matched the filters before
    /// `limit` was applied.
    pub total_matched: usize,
    /// `true` when more items matched than the limit returned.
    pub truncated: bool,
    /// Matched items, ranked by (name-match-bonus, hit-count desc,
    /// qualified-name asc).
    pub items: Vec<DocHit>,
}

/// One doc-comment match returned by the doc-comment search use case.
#[derive(Debug, Clone)]
pub struct DocHit {
    /// Normalised rustdoc kind (`struct`, `enum`, `trait`, `fn`,
    /// `macro`, `derive`, `module`, `type`, `constant`, `static`,
    /// `union`, `primitive`, `keyword`).
    pub kind: String,
    /// Fully-qualified item name as rustdoc renders it
    /// (e.g. `de::value::U8Deserializer`).
    pub name: String,
    /// URL-path tail relative to the crate's docs root. Pass this
    /// verbatim to `get_crate_docs.path` to read the full docs.
    pub path: String,
    /// Short excerpt of the doc comment centered on the first match,
    /// with leading / trailing `…` when truncated. ~200 chars.
    pub snippet: String,
}
