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
