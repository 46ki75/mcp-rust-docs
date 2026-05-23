use std::sync::Arc;

use crate::docs_rs::schema::DocsRsCrate;

/// Repository-tier projection of a docs.rs fetch.
///
/// The repository hands back the raw HTML body together with the final
/// URL reqwest landed on (after redirects). docs.rs's `/latest/`
/// redirect is the main reason we capture the final URL — callers
/// often want to surface the resolved version to the user.
#[derive(Debug, Clone)]
pub struct FetchCrateDocsRepositoryOutput {
    /// Final URL the server actually served the response from,
    /// post-redirect.
    pub final_url: String,
    /// Raw HTML body returned by docs.rs.
    pub html: String,
}

/// Repository-tier projection of a
/// `/crate/{name}/{version}/json/{format}.zst` fetch.
///
/// `crate_json` is the normalized, version-agnostic
/// [`DocsRsCrate`][crate::docs_rs::schema::DocsRsCrate]: the repository
/// dispatches the raw bytes through whichever `rustdoc-types` matches
/// the JSON's `format_version` (0.56 or 0.57), then translates the
/// result. The use case never sees either upstream version.
///
/// Wrapped in `Arc` because the parsed crate is large (hundreds of KB
/// to several MB for big crates) and the use case shouldn't pay to
/// clone it. Holding the parsed form here, not the raw bytes, means
/// the use case never re-parses on each query.
#[derive(Debug, Clone)]
pub struct FetchRustdocJsonRepositoryOutput {
    /// Final URL the server actually served the response from,
    /// post-redirect.
    pub final_url: String,
    /// Decompressed, deserialized, and normalized rustdoc JSON.
    pub crate_json: Arc<DocsRsCrate>,
}
