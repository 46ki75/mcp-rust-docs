/// Pre-validation arguments accepted by the use case.
///
/// The use case is what enforces "non-empty crate name, default
/// `version` to `latest`, reject paths containing `..` or leading
/// slashes" — so unlike
/// [`FetchCrateDocsRepositoryInput`][crate::docs_rs::repository::FetchCrateDocsRepositoryInput],
/// the optional fields here have not been resolved yet.
#[derive(Debug, Clone)]
pub struct FetchCrateDocsUseCaseInput {
    /// Crate name as published on crates.io (e.g. `tokio`,
    /// `tokio-util`). Hyphens are translated to underscores when
    /// assembling the docs.rs URL.
    pub crate_name: String,

    /// Optional version selector. `None` or `Some("latest")` resolves
    /// to docs.rs's `latest` alias. Otherwise expects a semver string
    /// docs.rs accepts (e.g. `1.40.0`).
    pub version: Option<String>,

    /// Optional URL-path tail relative to the crate's documentation
    /// root. Examples: `task/struct.JoinHandle.html`,
    /// `sync/index.html`. `None` fetches the crate root.
    pub path: Option<String>,
}

/// Pre-validation arguments for the symbol-search operation.
///
/// `query` is matched as a case-insensitive substring against the
/// fully-qualified item name (e.g. `de::value::U8Deserializer`).
/// `kinds` filters by the rustdoc-normalised kind (`struct`, `enum`,
/// `trait`, `fn`, …). `limit` caps the number of returned items; the
/// total match count is reported separately so callers know when
/// they've been truncated.
#[derive(Debug, Clone)]
pub struct SearchCrateSymbolsUseCaseInput {
    /// Crate name. Same normalisation as
    /// [`FetchCrateDocsUseCaseInput::crate_name`].
    pub crate_name: String,
    /// Optional version selector. `None`/`"latest"` resolve to
    /// docs.rs's `latest` alias.
    pub version: Option<String>,
    /// Optional substring filter. `None` or empty matches every item.
    pub query: Option<String>,
    /// Optional kind filter. Use case-insensitive normalised names
    /// (`struct`, `enum`, `trait`, `fn`, `macro`, `derive`,
    /// `attribute`, `type`, `module`, `constant`, `static`, `union`,
    /// `primitive`). Unknown kinds are silently ignored — the use
    /// case treats them as "match nothing", not an error, so callers
    /// can pass forward-compatible lists.
    pub kinds: Option<Vec<String>>,
    /// Optional cap on returned items. The use case defaults this to
    /// 50 and clamps to 500 to keep response sizes bounded.
    pub limit: Option<u32>,
}

/// Pre-validation arguments for the doc-comment full-text search.
///
/// Unlike [`SearchCrateSymbolsUseCaseInput`], `query` is required and
/// non-empty: an empty search pattern would return every item with a
/// doc comment. The match runs case-insensitively against each item's
/// rustdoc-JSON `docs` field — i.e. the raw doc comment Markdown, with
/// intra-doc links and code fences intact.
#[derive(Debug, Clone)]
pub struct SearchCrateDocsUseCaseInput {
    /// Crate name. Same normalisation as
    /// [`FetchCrateDocsUseCaseInput::crate_name`].
    pub crate_name: String,
    /// Optional version selector. `None`/`"latest"` resolve to
    /// docs.rs's `latest` alias.
    pub version: Option<String>,
    /// Required pattern. Case-insensitive substring matched against
    /// each item's doc-comment body. Empty / whitespace-only is
    /// rejected.
    pub query: String,
    /// Optional kind filter. Use case-insensitive normalised names
    /// (`struct`, `enum`, `trait`, `fn`, `macro`, `derive`, `module`,
    /// …). Unknown kinds match nothing; an empty list is treated as
    /// "no filter".
    pub kinds: Option<Vec<String>>,
    /// Optional cap on returned items. Defaults to 20, clamped to 100.
    pub limit: Option<u32>,
}
