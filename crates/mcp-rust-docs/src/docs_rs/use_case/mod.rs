/// Use case error type.
pub mod error;
/// Use case input types.
pub mod input;
/// Use case output types.
pub mod output;

use std::sync::Arc;

pub use self::error::DocsRsUseCaseError;
pub use self::input::FetchCrateDocsUseCaseInput;
pub use self::output::FetchCrateDocsUseCaseOutput;

use crate::docs_rs::repository::{
    DocsRsRepository, FetchCrateDocsRepositoryInput, FetchCrateDocsRepositoryOutput,
};

const DEFAULT_VERSION: &str = "latest";

/// crates.io caps published crate names at 64 ASCII characters.
/// Mirror that bound so we never build a multi-megabyte URL out of
/// pathological tool input.
const MAX_CRATE_NAME_LEN: usize = 64;

/// Same cap reused for `version` and `path`. docs.rs versions are
/// short semver strings; paths point at rustdoc HTML files whose
/// names are also short. A generous 256 covers every real case and
/// still bounds the URL size.
const MAX_VERSION_LEN: usize = 64;
const MAX_PATH_LEN: usize = 256;

/// Use case for fetching crate documentation from docs.rs.
///
/// Holds the repository behind `Arc<dyn>` so production wiring and
/// stub-backed unit tests share the same code path. All input
/// validation, URL assembly, and HTML→Markdown conversion live here
/// — the repository is dumb I/O and the tool layer is dumb DTO
/// translation.
pub struct DocsRsUseCase {
    repository: Arc<dyn DocsRsRepository>,
    base_url: Arc<str>,
}

impl DocsRsUseCase {
    /// Build a use case backed by the given repository and pointed at
    /// the given docs.rs base URL (e.g. `https://docs.rs`).
    pub fn new(repository: Arc<dyn DocsRsRepository>, base_url: impl Into<Arc<str>>) -> Self {
        Self {
            repository,
            base_url: base_url.into(),
        }
    }

    /// Validate and assemble the docs.rs URL, fetch the page, and
    /// convert the body to Markdown.
    #[tracing::instrument(skip(self))]
    pub async fn fetch_crate_docs(
        &self,
        input: FetchCrateDocsUseCaseInput,
    ) -> Result<FetchCrateDocsUseCaseOutput, DocsRsUseCaseError> {
        let crate_name = validate_crate_name(input.crate_name.trim())?;
        let version = match input.version.as_deref().map(str::trim) {
            None | Some("") => DEFAULT_VERSION.to_string(),
            Some(v) => validate_version(v)?,
        };
        let path = match input.path.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(p) => Some(validate_path(p)?),
        };

        let url = build_url(&self.base_url, &crate_name, &version, path.as_deref());

        let FetchCrateDocsRepositoryOutput { final_url, html } = self
            .repository
            .fetch_crate_docs(FetchCrateDocsRepositoryInput { url })
            .await?;

        let resolved_version = parse_version_from_url(&self.base_url, &crate_name, &final_url);
        let markdown = html_to_markdown(&html);

        Ok(FetchCrateDocsUseCaseOutput {
            crate_name,
            resolved_version,
            final_url,
            markdown,
        })
    }
}

fn validate_crate_name(name: &str) -> Result<String, DocsRsUseCaseError> {
    if name.is_empty() {
        return Err(DocsRsUseCaseError::InvalidInput(
            "crate name must not be empty".into(),
        ));
    }
    if name.len() > MAX_CRATE_NAME_LEN {
        return Err(DocsRsUseCaseError::InvalidInput(format!(
            "crate name longer than {MAX_CRATE_NAME_LEN} characters"
        )));
    }
    // crates.io enforces `[A-Za-z0-9_-]+`; reject anything outside that
    // before we go anywhere near the network, so we never accidentally
    // smuggle path-traversal segments or query strings into the URL.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(DocsRsUseCaseError::InvalidInput(format!(
            "crate name contains disallowed characters: {name:?}"
        )));
    }
    // docs.rs URLs are case-sensitive but crates.io is case-insensitive
    // on lookup. The model often capitalises crate names; quietly
    // normalising avoids a confusing 404 round-trip.
    Ok(name.to_ascii_lowercase())
}

fn validate_version(version: &str) -> Result<String, DocsRsUseCaseError> {
    if version.len() > MAX_VERSION_LEN {
        return Err(DocsRsUseCaseError::InvalidInput(format!(
            "version longer than {MAX_VERSION_LEN} characters"
        )));
    }
    // docs.rs accepts `latest`, semver, and a few semver-range
    // sigils. Reject anything that would let a caller break out of
    // the version path segment, smuggle in a query/fragment, or
    // inject control characters.
    if version.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(DocsRsUseCaseError::InvalidInput(
            "version must not contain whitespace or control characters".into(),
        ));
    }
    for forbidden in ['/', '\\', '?', '#'] {
        if version.contains(forbidden) {
            return Err(DocsRsUseCaseError::InvalidInput(format!(
                "version must not contain {forbidden:?}"
            )));
        }
    }
    if version.contains("..") {
        return Err(DocsRsUseCaseError::InvalidInput(
            "version must not contain `..`".into(),
        ));
    }
    Ok(version.to_string())
}

fn validate_path(path: &str) -> Result<String, DocsRsUseCaseError> {
    if path.len() > MAX_PATH_LEN {
        return Err(DocsRsUseCaseError::InvalidInput(format!(
            "path longer than {MAX_PATH_LEN} characters"
        )));
    }
    if path.starts_with('/') {
        return Err(DocsRsUseCaseError::InvalidInput(
            "path must be relative (no leading `/`)".into(),
        ));
    }
    if path.contains('\\') {
        return Err(DocsRsUseCaseError::InvalidInput(
            "path must not contain backslashes".into(),
        ));
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(DocsRsUseCaseError::InvalidInput(
            "path must not contain control characters".into(),
        ));
    }
    if path.contains('?') || path.contains('#') {
        return Err(DocsRsUseCaseError::InvalidInput(
            "path must not contain `?` or `#` (no query or fragment)".into(),
        ));
    }
    if path.contains("//") {
        return Err(DocsRsUseCaseError::InvalidInput(
            "path must not contain empty segments (`//`)".into(),
        ));
    }
    // Reject parent-directory traversals — both raw and the common
    // URL-encoded variants. The encoded forms matter because some
    // upstream proxies normalise `%2e%2e` and `%2f` back to `..`/`/`
    // before forwarding.
    let lower = path.to_ascii_lowercase();
    if path.split('/').any(|segment| segment == "..")
        || lower.contains("%2e%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
    {
        return Err(DocsRsUseCaseError::InvalidInput(
            "path must not contain `..` segments or their URL-encoded equivalents".into(),
        ));
    }
    Ok(path.to_string())
}

fn build_url(base_url: &str, crate_name: &str, version: &str, path: Option<&str>) -> String {
    let base = base_url.trim_end_matches('/');
    let lib_name = crate_name.replace('-', "_");
    match path {
        Some(p) => format!("{base}/{crate_name}/{version}/{lib_name}/{p}"),
        None => format!("{base}/{crate_name}/{version}/{lib_name}/"),
    }
}

/// Pull the actual version out of the final URL after docs.rs
/// resolves `/latest/`. The URL shape we expect is
/// `{base}/{crate}/{version}/{lib_name}/...`; if it doesn't match
/// (mirror with a different layout, error page rendered at a top-level
/// URL, etc.) we return `None` rather than guessing.
fn parse_version_from_url(base_url: &str, crate_name: &str, final_url: &str) -> Option<String> {
    let prefix = format!("{}/{crate_name}/", base_url.trim_end_matches('/'));
    let rest = final_url.strip_prefix(&prefix)?;
    let version = rest.split('/').next()?;
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// Pull the rustdoc body content out of the surrounding chrome
/// (sidebar, search box, footer). rustdoc emits exactly one `<main>`
/// element per page wrapping the meaningful prose, so a simple
/// substring slice is reliable enough and avoids pulling in a DOM
/// parser. If the markers aren't found (mirror with a custom layout,
/// an error page, or non-rustdoc HTML) we fall back to the full body
/// so the caller still gets *something*.
fn extract_main_content(html: &str) -> &str {
    let Some(open_idx) = html.find("<main") else {
        return html;
    };
    let after_open = &html[open_idx..];
    let Some(close_rel) = after_open.find("</main>") else {
        return html;
    };
    // `</main>` is 7 chars; include it so the closing tag balances
    // and html2md doesn't see a half-open element.
    &after_open[..close_rel + "</main>".len()]
}

fn html_to_markdown(html: &str) -> String {
    let body = extract_main_content(html);
    // `commonmark = true` keeps the output compatible with most
    // Markdown renderers (avoids the GitHub-only extensions the
    // library emits by default).
    html2md::rewrite_html(body, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs_rs::repository::{
        DocsRsRepositoryError, DocsRsRepositoryStub, FetchCrateDocsRepositoryOutput,
    };

    const BASE_URL: &str = "https://docs.rs";

    fn use_case_with(stub: Arc<DocsRsRepositoryStub>) -> DocsRsUseCase {
        DocsRsUseCase::new(stub, BASE_URL)
    }

    #[tokio::test]
    async fn fetch_defaults_version_to_latest_and_omits_path() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/tokio/1.40.0/tokio/".into(),
            html: "<main><h1>tokio</h1></main>".into(),
        }))
        .await;
        let use_case = use_case_with(stub.clone());

        let out = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: " tokio ".into(),
                version: None,
                path: None,
            })
            .await?;

        assert_eq!(
            stub.last_seen_url().await.as_deref(),
            Some("https://docs.rs/tokio/latest/tokio/"),
        );
        assert_eq!(out.crate_name, "tokio");
        assert_eq!(out.resolved_version.as_deref(), Some("1.40.0"));
        assert!(out.markdown.contains("tokio"));
        Ok(())
    }

    #[tokio::test]
    async fn fetch_translates_hyphens_to_underscores_in_lib_name() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/tokio-util/0.7.10/tokio_util/".into(),
            html: "<p>tokio-util</p>".into(),
        }))
        .await;
        let use_case = use_case_with(stub.clone());

        let _ = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "tokio-util".into(),
                version: Some("0.7.10".into()),
                path: None,
            })
            .await?;

        assert_eq!(
            stub.last_seen_url().await.as_deref(),
            Some("https://docs.rs/tokio-util/0.7.10/tokio_util/"),
        );
        Ok(())
    }

    #[tokio::test]
    async fn fetch_appends_path_tail() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html".into(),
            html: "<h1>JoinHandle</h1>".into(),
        }))
        .await;
        let use_case = use_case_with(stub.clone());

        let _ = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "tokio".into(),
                version: None,
                path: Some("task/struct.JoinHandle.html".into()),
            })
            .await?;

        assert_eq!(
            stub.last_seen_url().await.as_deref(),
            Some("https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html"),
        );
        Ok(())
    }

    #[tokio::test]
    async fn fetch_rejects_empty_crate_name() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        let err = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "  ".into(),
                version: None,
                path: None,
            })
            .await
            .expect_err("expected validation error");

        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn fetch_rejects_crate_name_with_path_separator() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        let err = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "../etc/passwd".into(),
                version: None,
                path: None,
            })
            .await
            .expect_err("expected validation error");

        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn fetch_rejects_path_with_parent_traversal() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        let err = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "tokio".into(),
                version: None,
                path: Some("../../etc/passwd".into()),
            })
            .await
            .expect_err("expected validation error");

        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn fetch_lowercases_crate_name_for_url() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/tokio/latest/tokio/".into(),
            html: String::new(),
        }))
        .await;
        let use_case = use_case_with(stub.clone());

        let out = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "Tokio".into(),
                version: None,
                path: None,
            })
            .await?;

        assert_eq!(out.crate_name, "tokio");
        assert_eq!(
            stub.last_seen_url().await.as_deref(),
            Some("https://docs.rs/tokio/latest/tokio/"),
        );
        Ok(())
    }

    #[tokio::test]
    async fn fetch_rejects_overlong_crate_name() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        let err = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "a".repeat(65),
                version: None,
                path: None,
            })
            .await
            .expect_err("expected length cap to fire");

        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn fetch_rejects_version_with_query_or_fragment() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        for bad in ["latest?x=1", "latest#frag", "1.0\\0", "1 0"] {
            let err = use_case
                .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                    crate_name: "tokio".into(),
                    version: Some(bad.into()),
                    path: None,
                })
                .await
                .expect_err("expected validation error");
            assert!(
                matches!(err, DocsRsUseCaseError::InvalidInput(_)),
                "expected InvalidInput for version {bad:?}, got {err:?}",
            );
        }
    }

    #[tokio::test]
    async fn fetch_rejects_path_with_backslash_or_double_slash() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        for bad in ["foo\\bar", "foo//bar", "task/%2e%2e/x", "%2fmod/x"] {
            let err = use_case
                .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                    crate_name: "tokio".into(),
                    version: None,
                    path: Some(bad.into()),
                })
                .await
                .expect_err("expected validation error");
            assert!(
                matches!(err, DocsRsUseCaseError::InvalidInput(_)),
                "expected InvalidInput for path {bad:?}, got {err:?}",
            );
        }
    }

    #[tokio::test]
    async fn fetch_rejects_absolute_path() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        let err = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "tokio".into(),
                version: None,
                path: Some("/etc/passwd".into()),
            })
            .await
            .expect_err("expected validation error");

        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn fetch_bubbles_not_found_from_repository() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Err(DocsRsRepositoryError::NotFound {
            url: "https://docs.rs/nonexistent-crate-zzz/latest/nonexistent_crate_zzz/".into(),
        }))
        .await;
        let use_case = use_case_with(stub);

        let err = use_case
            .fetch_crate_docs(FetchCrateDocsUseCaseInput {
                crate_name: "nonexistent-crate-zzz".into(),
                version: None,
                path: None,
            })
            .await
            .expect_err("expected upstream not-found");

        assert!(matches!(
            err,
            DocsRsUseCaseError::Repository(DocsRsRepositoryError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn html_to_markdown_strips_tags() {
        let md = html_to_markdown("<h1>Hello</h1><p>world <strong>here</strong></p>");
        assert!(md.contains("Hello"), "expected heading in markdown: {md:?}");
        assert!(md.contains("world"), "expected paragraph text: {md:?}");
        assert!(
            !md.contains("<h1>") && !md.contains("<strong>"),
            "html tags should be stripped: {md:?}",
        );
    }

    #[test]
    fn extract_main_drops_surrounding_chrome() {
        let html = r#"<html><body>
            <nav>SIDEBAR LINKS</nav>
            <main><p>Body content</p></main>
            <footer>FOOTER LINKS</footer>
        </body></html>"#;
        let extracted = extract_main_content(html);
        assert!(extracted.contains("Body content"));
        assert!(
            !extracted.contains("SIDEBAR") && !extracted.contains("FOOTER"),
            "chrome leaked into extracted slice: {extracted}",
        );
    }

    #[test]
    fn extract_main_falls_back_to_full_html_without_marker() {
        let html = "<div>no main here</div>";
        assert_eq!(extract_main_content(html), html);
    }

    #[test]
    fn parse_version_handles_redirected_latest() {
        let v = parse_version_from_url(
            BASE_URL,
            "tokio",
            "https://docs.rs/tokio/1.40.0/tokio/index.html",
        );
        assert_eq!(v.as_deref(), Some("1.40.0"));
    }

    #[test]
    fn parse_version_returns_none_for_unexpected_shape() {
        let v = parse_version_from_url(BASE_URL, "tokio", "https://docs.rs/about");
        assert_eq!(v, None);
    }
}
