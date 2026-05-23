/// Arguments sent down to the repository layer. Post-validation: the
/// use case has already trimmed the crate name, defaulted the version,
/// and resolved the URL-path tail.
#[derive(Debug, Clone)]
pub struct FetchCrateDocsRepositoryInput {
    /// Fully-assembled docs.rs URL to fetch. The use case is what
    /// builds this — the repository is dumb I/O.
    pub url: String,
}
