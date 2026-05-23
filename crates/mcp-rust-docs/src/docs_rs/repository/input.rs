/// Arguments sent down to the repository layer. Post-validation: the
/// use case has already trimmed the crate name, defaulted the version,
/// and resolved the URL-path tail.
#[derive(Debug, Clone)]
pub struct FetchCrateDocsRepositoryInput {
    /// Fully-assembled docs.rs URL to fetch. The use case is what
    /// builds this — the repository is dumb I/O.
    pub url: String,
}

/// Arguments for [`DocsRsRepository::fetch_rustdoc_json`].
/// Post-validation — the use case has built the full
/// `https://docs.rs/crate/{name}/{version}/json/{format_version}.zst`
/// URL before handing it down. The use case is also where the
/// format-version fallback chain lives: a single `fetch_rustdoc_json`
/// call corresponds to one specific format-version build.
///
/// [`DocsRsRepository::fetch_rustdoc_json`]:
/// crate::docs_rs::repository::DocsRsRepository::fetch_rustdoc_json
#[derive(Debug, Clone)]
pub struct FetchRustdocJsonRepositoryInput {
    /// Fully-assembled docs.rs URL pointing at the zstd-compressed
    /// rustdoc JSON. The repository decompresses it, dispatches to the
    /// `rustdoc-types` crate matching the JSON's `format_version`, and
    /// normalizes the result.
    pub url: String,
}
