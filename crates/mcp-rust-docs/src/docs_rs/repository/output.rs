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
