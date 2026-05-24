/// Failures that can happen at the HTTP boundary against docs.rs.
///
/// `Network` is infrastructure (we never reached or never read the
/// response). `NotFound` is a 404 specifically — broken out from the
/// generic `UpstreamStatus` because the use case maps it to a
/// caller-facing "did you typo the crate name / path?" message rather
/// than a generic upstream failure.
#[derive(Debug, thiserror::Error)]
pub enum DocsRsRepositoryError {
    /// `reqwest` could not complete the request — DNS, TLS, connection
    /// reset, body read failure, etc.
    #[error("HTTP request to docs.rs failed: {0}")]
    Network(#[from] reqwest::Error),

    /// docs.rs returned 404. The URL is captured so the caller can
    /// echo it back to the user (the most common cause is a wrong
    /// crate name or item path).
    #[error("docs.rs returned 404 for {url}")]
    NotFound {
        /// URL that returned 404.
        url: String,
    },

    /// docs.rs returned a non-2xx, non-404 response. Body kept for
    /// diagnostics (usually an HTML error page).
    #[error("docs.rs returned HTTP {status} for {url}")]
    UpstreamStatus {
        /// HTTP status code returned by docs.rs.
        status: reqwest::StatusCode,
        /// URL that triggered the failure.
        url: String,
        /// Raw response body, kept for diagnostics.
        body: String,
    },

    /// The compressed rustdoc-JSON payload (or its decompressed form)
    /// exceeded the configured size cap. The cap exists so a single
    /// huge crate (or a malicious upstream) can't exhaust memory.
    #[error("docs.rs rustdoc JSON for {url} exceeds {limit_bytes}-byte cap")]
    PayloadTooLarge {
        /// URL whose payload exceeded the cap.
        url: String,
        /// The cap that fired, in bytes.
        limit_bytes: usize,
    },

    /// zstd decompression of the rustdoc-JSON payload failed.
    #[error("failed to decompress rustdoc JSON from {url}: {source}")]
    Decompression {
        /// URL that produced the unreadable payload.
        url: String,
        /// Wrapped zstd error.
        #[source]
        source: std::io::Error,
    },

    /// The decompressed payload couldn't be deserialized by the
    /// dispatched `rustdoc-types` crate — malformed JSON, missing
    /// required fields, or a shape that version doesn't model. A
    /// `format_version` skew against the set of supported versions is
    /// caught *before* the full deserialize attempt by the dispatch
    /// path and surfaces through [`Self::FormatVersionUnsupported`],
    /// not this variant.
    #[error("failed to parse rustdoc JSON from {url}: {source}")]
    InvalidRustdocJson {
        /// URL whose payload failed to parse.
        url: String,
        /// Wrapped serde error.
        #[source]
        source: serde_json::Error,
    },

    /// The payload's `format_version` isn't one this build of
    /// `mcp-rust-docs` ships a deserializer for. Broken out from
    /// [`Self::InvalidRustdocJson`] because the model needs to know
    /// this is an upstream-vs-tooling skew, not a corrupt response.
    ///
    /// The repository ships dispatch arms for each format version it
    /// supports (currently 56 and 57 via the cargo-renamed
    /// `rustdoc-types-56` / `rustdoc-types`); anything outside that set
    /// surfaces here so the user can decide whether to upgrade the
    /// tool or wait for docs.rs to rebuild.
    #[error(
        "rustdoc JSON from {url} has format_version {actual}, but this build of mcp-rust-docs \
         only supports {supported:?}"
    )]
    FormatVersionUnsupported {
        /// URL the JSON was fetched from.
        url: String,
        /// `format_version` reported by the payload.
        actual: u32,
        /// Format versions this build can deserialize, in dispatch
        /// preference order.
        supported: Vec<u32>,
    },
}
