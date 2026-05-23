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
