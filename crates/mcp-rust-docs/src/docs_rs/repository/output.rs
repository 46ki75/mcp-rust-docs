use std::sync::Arc;

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

/// Repository-tier projection of a `/crate/{name}/{version}/json.zst`
/// fetch.
///
/// `crate_json` is wrapped in `Arc` because `rustdoc_types::Crate` is
/// large (hundreds of KB to several MB for big crates) and the use
/// case shouldn't pay to clone it. Holding the parsed form here, not
/// the raw bytes, means the use case never re-parses on each query.
#[derive(Debug, Clone)]
pub struct FetchRustdocJsonRepositoryOutput {
    /// Final URL the server actually served the response from,
    /// post-redirect.
    pub final_url: String,
    /// Decompressed, deserialized rustdoc JSON.
    pub crate_json: Arc<rustdoc_types::Crate>,
}
