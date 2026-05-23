/// Arguments sent down to the repository layer. Post-validation: the
/// use case has already trimmed the crate name, defaulted the version,
/// and resolved the URL-path tail.
#[derive(Debug, Clone)]
pub struct FetchCrateDocsRepositoryInput {
    /// Fully-assembled docs.rs URL to fetch. The use case is what
    /// builds this — the repository is dumb I/O.
    pub url: String,
}

/// Arguments for [`DocsRsRepository::fetch_rustdoc_json`]. Post-validation
/// — the use case has built the full `https://docs.rs/crate/{name}/{version}/json.zst`
/// URL before handing it down.
///
/// [`DocsRsRepository::fetch_rustdoc_json`]:
/// crate::docs_rs::repository::DocsRsRepository::fetch_rustdoc_json
#[derive(Debug, Clone)]
pub struct FetchRustdocJsonRepositoryInput {
    /// Fully-assembled docs.rs URL pointing at the zstd-compressed
    /// rustdoc JSON. The repository decompresses and deserializes it.
    pub url: String,
}
