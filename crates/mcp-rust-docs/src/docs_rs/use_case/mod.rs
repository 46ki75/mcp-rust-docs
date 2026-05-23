/// Use case error type.
pub mod error;
/// Use case input types.
pub mod input;
/// Use case output types.
pub mod output;

use std::sync::Arc;

pub use self::error::DocsRsUseCaseError;
pub use self::input::{
    FetchCrateDocsUseCaseInput, GrepCrateDocsUseCaseInput, SearchCrateSymbolsUseCaseInput,
};
pub use self::output::{
    DocHit, FetchCrateDocsUseCaseOutput, GrepCrateDocsUseCaseOutput,
    SearchCrateSymbolsUseCaseOutput, SymbolEntry,
};

use crate::docs_rs::repository::{
    DocsRsRepository, FetchCrateDocsRepositoryInput, FetchCrateDocsRepositoryOutput,
    FetchRustdocJsonRepositoryInput, FetchRustdocJsonRepositoryOutput,
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

/// Cap on the symbol-search query string. Substring matching against
/// item names rarely needs more than a handful of characters; bounding
/// this stops a runaway tool input from forcing us to scan a long
/// pattern across every all.html entry.
const MAX_SYMBOL_QUERY_LEN: usize = 128;

/// Default and maximum result count for symbol search. The cap keeps
/// response sizes bounded so a query like `""` against `tokio` (which
/// has ~1k items) can't return them all in one shot.
const DEFAULT_SYMBOL_LIMIT: u32 = 50;
const MAX_SYMBOL_LIMIT: u32 = 500;

/// Default and maximum result count for the doc-comment grep tool.
/// Lower than the symbol-search cap because each hit ships a ~200-char
/// snippet — the per-item payload is roughly 3x larger.
const DEFAULT_GREP_LIMIT: u32 = 20;
const MAX_GREP_LIMIT: u32 = 100;

/// Cap on the grep query string. Generous enough for a short phrase
/// ("zero-copy deserialization") but bounded so a runaway tool input
/// can't force us to scan a kilobyte pattern across thousands of
/// doc-comment bodies.
const MAX_GREP_QUERY_LEN: usize = 256;

/// Target length, in chars, of the snippet returned per doc-comment
/// hit. Roughly two sentences either side of the match.
const SNIPPET_TARGET_CHARS: usize = 200;

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

    /// Fetch `all.html` for the crate, parse the public-item index,
    /// then filter by `query` (case-insensitive substring on the
    /// qualified name) and `kinds`. The matched list is truncated to
    /// `limit`; `total_matched` reports the pre-truncation count so
    /// callers know when to narrow the query.
    #[tracing::instrument(skip(self))]
    pub async fn search_crate_symbols(
        &self,
        input: SearchCrateSymbolsUseCaseInput,
    ) -> Result<SearchCrateSymbolsUseCaseOutput, DocsRsUseCaseError> {
        let crate_name = validate_crate_name(input.crate_name.trim())?;
        let version = match input.version.as_deref().map(str::trim) {
            None | Some("") => DEFAULT_VERSION.to_string(),
            Some(v) => validate_version(v)?,
        };
        let query = match input.query.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(q) => Some(validate_symbol_query(q)?),
        };
        // An empty `kinds` array means the caller didn't intend to
        // filter — collapse to `None` so we don't silently drop every
        // item and confuse the caller with `total_matched: 0`.
        let kinds: Option<Vec<String>> = input
            .kinds
            .map(|ks| {
                ks.into_iter()
                    .map(|k| k.trim().to_ascii_lowercase())
                    .filter(|k| !k.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|ks: &Vec<String>| !ks.is_empty());
        let limit = input
            .limit
            .unwrap_or(DEFAULT_SYMBOL_LIMIT)
            .clamp(1, MAX_SYMBOL_LIMIT) as usize;

        let url = build_all_html_url(&self.base_url, &crate_name, &version);

        let FetchCrateDocsRepositoryOutput { final_url, html } = self
            .repository
            .fetch_crate_docs(FetchCrateDocsRepositoryInput { url })
            .await?;

        let resolved_version = parse_version_from_url(&self.base_url, &crate_name, &final_url);

        let all_entries = parse_all_html(&html);
        let mut matched: Vec<SymbolEntry> = all_entries
            .into_iter()
            .filter(|entry| match_filters(entry, query.as_deref(), kinds.as_deref()))
            .collect();
        let total_matched = matched.len();
        let truncated = matched.len() > limit;
        matched.truncate(limit);

        Ok(SearchCrateSymbolsUseCaseOutput {
            crate_name,
            resolved_version,
            total_matched,
            truncated,
            items: matched,
        })
    }

    /// Fetch the crate's rustdoc JSON, walk every documented item, and
    /// return the ones whose doc-comment body contains `query`
    /// (case-insensitive substring). Results are ranked by
    /// item-name-match bonus, then hit count, then qualified name.
    ///
    /// Unlike [`Self::search_crate_symbols`] this requires a
    /// non-empty query — "grep with no pattern" would return every
    /// documented item in the crate, which is not useful.
    #[tracing::instrument(skip(self))]
    pub async fn grep_crate_docs(
        &self,
        input: GrepCrateDocsUseCaseInput,
    ) -> Result<GrepCrateDocsUseCaseOutput, DocsRsUseCaseError> {
        let crate_name = validate_crate_name(input.crate_name.trim())?;
        let version = match input.version.as_deref().map(str::trim) {
            None | Some("") => DEFAULT_VERSION.to_string(),
            Some(v) => validate_version(v)?,
        };
        let query = validate_grep_query(input.query.trim())?;
        let kinds: Option<Vec<String>> = input
            .kinds
            .map(|ks| {
                ks.into_iter()
                    .map(|k| k.trim().to_ascii_lowercase())
                    .filter(|k| !k.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|ks: &Vec<String>| !ks.is_empty());
        let limit = input
            .limit
            .unwrap_or(DEFAULT_GREP_LIMIT)
            .clamp(1, MAX_GREP_LIMIT) as usize;

        let url = build_rustdoc_json_url(&self.base_url, &crate_name, &version);

        let FetchRustdocJsonRepositoryOutput {
            final_url,
            crate_json,
        } = self
            .repository
            .fetch_rustdoc_json(FetchRustdocJsonRepositoryInput { url })
            .await?;

        let resolved_version = parse_rustdoc_json_version(&self.base_url, &crate_name, &final_url);

        let query_lower = query.to_lowercase();
        let mut hits: Vec<RankedHit> = Vec::new();

        for (id, item) in &crate_json.index {
            let docs = match item.docs.as_deref() {
                Some(d) if !d.is_empty() => d,
                _ => continue,
            };
            // Only items present in `paths` have addressable doc pages.
            // Skip impls / fields / variants — their docs live on a
            // parent page that the caller can find separately.
            let summary = match crate_json.paths.get(id) {
                Some(s) => s,
                None => continue,
            };
            let (kind, path) = match rustdoc_kind_and_path(summary) {
                Some(parts) => parts,
                None => continue,
            };
            if let Some(kinds) = kinds.as_deref()
                && !kinds.iter().any(|k| k == &kind)
            {
                continue;
            }

            let docs_lower = docs.to_lowercase();
            let hit_count = count_substr(&docs_lower, &query_lower);
            if hit_count == 0 {
                continue;
            }

            // `summary.path[0]` is the crate's lib name; the human-
            // friendly qualified name skips it so the model sees
            // `de::value::U8Deserializer`, not `serde::de::value::U8Deserializer`.
            let qualified_name = qualified_name_from_summary(summary);
            let name_match = item
                .name
                .as_deref()
                .map(|n| n.to_lowercase().contains(&query_lower))
                .unwrap_or(false);
            let snippet = snippet_around(docs, &docs_lower, &query_lower, SNIPPET_TARGET_CHARS);

            hits.push(RankedHit {
                kind,
                qualified_name,
                path,
                snippet,
                hit_count,
                name_match,
            });
        }

        // Sort: name matches first, then more hits first, then
        // qualified-name asc for a stable order.
        hits.sort_by(|a, b| {
            b.name_match
                .cmp(&a.name_match)
                .then_with(|| b.hit_count.cmp(&a.hit_count))
                .then_with(|| a.qualified_name.cmp(&b.qualified_name))
        });

        let total_matched = hits.len();
        let truncated = total_matched > limit;
        hits.truncate(limit);

        let items = hits
            .into_iter()
            .map(|h| DocHit {
                kind: h.kind,
                name: h.qualified_name,
                path: h.path,
                snippet: h.snippet,
            })
            .collect();

        Ok(GrepCrateDocsUseCaseOutput {
            crate_name,
            resolved_version,
            total_matched,
            truncated,
            items,
        })
    }
}

/// Internal pre-sort representation. Carries the ranking signals so
/// we can sort once after collection rather than computing them in
/// the `sort_by` comparator (each item already paid the cost during
/// the walk).
struct RankedHit {
    kind: String,
    qualified_name: String,
    path: String,
    snippet: String,
    hit_count: usize,
    name_match: bool,
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

fn build_all_html_url(base_url: &str, crate_name: &str, version: &str) -> String {
    build_url(base_url, crate_name, version, Some("all.html"))
}

/// Build the docs.rs rustdoc-JSON URL. Shape:
/// `{base}/crate/{crate}/{version}/json.zst`. Note the leading
/// `/crate/` segment — this is a *different* docs.rs endpoint family
/// from the rendered-HTML routes used by `fetch_crate_docs` and is
/// NOT under the lib-name path. The `.zst` suffix selects the
/// zstd-compressed variant (much smaller than `/json`).
fn build_rustdoc_json_url(base_url: &str, crate_name: &str, version: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/crate/{crate_name}/{version}/json.zst")
}

/// Parse the concrete version out of the redirected rustdoc-JSON URL.
/// docs.rs serves `latest` by redirect, same as the HTML routes; the
/// final URL is `{base}/crate/{crate}/{version}/json.zst`, so the
/// version sits in the third path segment after the base.
fn parse_rustdoc_json_version(base_url: &str, crate_name: &str, final_url: &str) -> Option<String> {
    let prefix = format!("{}/crate/{crate_name}/", base_url.trim_end_matches('/'));
    let rest = final_url.strip_prefix(&prefix)?;
    let version = rest.split('/').next()?;
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

fn validate_grep_query(query: &str) -> Result<String, DocsRsUseCaseError> {
    if query.is_empty() {
        return Err(DocsRsUseCaseError::InvalidInput(
            "query must not be empty (grep needs a pattern)".into(),
        ));
    }
    if query.len() > MAX_GREP_QUERY_LEN {
        return Err(DocsRsUseCaseError::InvalidInput(format!(
            "query longer than {MAX_GREP_QUERY_LEN} characters"
        )));
    }
    if query.chars().any(|c| c.is_control()) {
        return Err(DocsRsUseCaseError::InvalidInput(
            "query must not contain control characters".into(),
        ));
    }
    Ok(query.to_string())
}

/// Map a [`rustdoc_types::ItemSummary`] onto the normalised kind
/// string and the URL-path tail under the crate's docs root.
///
/// Returns `None` for kinds that don't have a dedicated page (impls,
/// fields, variants, use-statements, etc.) or kinds we don't model.
/// Filtering on `None` keeps the grep results to items the caller can
/// actually open with `get_crate_docs`.
fn rustdoc_kind_and_path(summary: &rustdoc_types::ItemSummary) -> Option<(String, String)> {
    use rustdoc_types::ItemKind;
    // `path[0]` is the crate lib name; drop it so the URL becomes
    // relative to the crate docs root. `last` is the item's own name.
    let mut segments = summary.path.iter();
    let _crate = segments.next()?;
    let parents: Vec<&str> = segments.clone().map(String::as_str).collect();
    if parents.is_empty() {
        // The summary refers to the crate root itself; it's the same
        // page as `get_crate_docs` with no `path` argument, so callers
        // don't gain anything from a grep hit here. Skip.
        return None;
    }
    let last = parents.last().copied()?;
    let parent_segments = &parents[..parents.len() - 1];
    let dir = if parent_segments.is_empty() {
        String::new()
    } else {
        format!("{}/", parent_segments.join("/"))
    };

    let (kind_str, filename) = match &summary.kind {
        ItemKind::Module => ("module", format!("{}{last}/index.html", dir)),
        ItemKind::Struct => ("struct", format!("{dir}struct.{last}.html")),
        ItemKind::Union => ("union", format!("{dir}union.{last}.html")),
        ItemKind::Enum => ("enum", format!("{dir}enum.{last}.html")),
        ItemKind::Function => ("fn", format!("{dir}fn.{last}.html")),
        ItemKind::TypeAlias => ("type", format!("{dir}type.{last}.html")),
        ItemKind::Constant => ("constant", format!("{dir}constant.{last}.html")),
        ItemKind::Trait => ("trait", format!("{dir}trait.{last}.html")),
        ItemKind::TraitAlias => ("traitalias", format!("{dir}traitalias.{last}.html")),
        ItemKind::Static => ("static", format!("{dir}static.{last}.html")),
        ItemKind::Macro => ("macro", format!("{dir}macro.{last}.html")),
        ItemKind::ProcDerive => ("derive", format!("{dir}derive.{last}.html")),
        ItemKind::ProcAttribute => ("attribute", format!("{dir}attr.{last}.html")),
        // `Attribute` (core's built-in attribute docs, e.g.
        // `#[no_mangle]`) uses the same `attr.{name}.html` URL shape as
        // `ProcAttribute`. Bundled under the same `"attribute"` kind so
        // a `kinds: ["attribute"]` filter covers both.
        ItemKind::Attribute => ("attribute", format!("{dir}attr.{last}.html")),
        ItemKind::Primitive => ("primitive", format!("{dir}primitive.{last}.html")),
        ItemKind::Keyword => ("keyword", format!("{dir}keyword.{last}.html")),
        // Items without a dedicated page (impls, fields, variants,
        // assoc-types/consts, use-statements, extern-crate /
        // extern-type) are skipped.
        _ => return None,
    };
    Some((kind_str.to_string(), filename))
}

/// Render the qualified name shown to the user. Drops the crate lib
/// name (path[0]) so output matches what `search_crate_symbols`
/// returns — `de::value::U8Deserializer`, not
/// `serde::de::value::U8Deserializer`.
fn qualified_name_from_summary(summary: &rustdoc_types::ItemSummary) -> String {
    summary
        .path
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>()
        .join("::")
}

/// Count non-overlapping occurrences of `needle` in `haystack`. Both
/// sides are expected to be already-lowercased when called from
/// `grep_crate_docs`; this function is byte-substring based and so
/// behaves the same on any input where caller lowercasing is
/// consistent.
fn count_substr(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        count += 1;
        start += rel + needle.len();
    }
    count
}

/// Build a snippet roughly `target_chars` wide, centered on the first
/// occurrence of `needle_lower` within `docs_lower`. Returns the slice
/// from the *original* `docs` string (preserving case) with `…` markers
/// on either side when the slice doesn't cover the full body. Snaps to
/// char boundaries so we never split a multi-byte sequence.
fn snippet_around(docs: &str, docs_lower: &str, needle_lower: &str, target_chars: usize) -> String {
    let hit_lower = docs_lower.find(needle_lower).unwrap_or(0);
    // `hit_lower` is a byte position in `docs_lower`, not in `docs`.
    // Translate it back: when a char's lowercase form has a different
    // byte length than the original (e.g. `İ` U+0130 is 2 bytes but
    // lowercases to `i\u{307}` which is 3) the two indices diverge.
    let hit = translate_lower_index_to_docs(docs, hit_lower);
    let half = target_chars / 2;
    let start_byte = floor_char_boundary(docs, hit.saturating_sub(half));
    // The trailing window uses `needle_lower.len()` (lowercase-coordinate
    // byte length); the `± half` slack absorbs any drift, so the match
    // is always visible even if the snippet is slightly off-center on
    // the trailing side.
    let end_target = hit.saturating_add(needle_lower.len()).saturating_add(half);
    let end_byte = ceil_char_boundary(docs, end_target.min(docs.len()));

    let prefix = if start_byte > 0 { "…" } else { "" };
    let suffix = if end_byte < docs.len() { "…" } else { "" };
    // `replace('\n', " ")` keeps the snippet on one line so the
    // JSON-rendered tool result stays readable in MCP Inspector
    // panes that don't pretty-print embedded newlines.
    let body = docs[start_byte..end_byte]
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string();
    format!("{prefix}{body}{suffix}")
}

/// Translate a byte index from the lowercased form of `docs` back to
/// the corresponding byte index in the original `docs`.
///
/// Walks `docs` char by char, accumulating both the original byte
/// length and the lowercased byte length in parallel. When the
/// lowercased counter reaches `hit_lower`, the original counter holds
/// the matching offset. Needed because some chars change byte length
/// under [`str::to_lowercase`] (`İ` 2 → 3 bytes, `ẞ` 3 → 2 bytes), so
/// the two byte coordinate spaces are not interchangeable.
fn translate_lower_index_to_docs(docs: &str, hit_lower: usize) -> usize {
    let mut docs_byte = 0usize;
    let mut lower_byte = 0usize;
    for ch in docs.chars() {
        if lower_byte >= hit_lower {
            break;
        }
        docs_byte += ch.len_utf8();
        lower_byte += ch.to_lowercase().map(|c| c.len_utf8()).sum::<usize>();
    }
    docs_byte
}

/// `str::floor_char_boundary` is still nightly-only; this is the
/// stable equivalent. Walks backwards from `i` until landing on a
/// char boundary, capped at `s.len()`.
fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Stable analogue of `str::ceil_char_boundary`. Walks forwards from
/// `i` until landing on a char boundary, capped at `s.len()`.
fn ceil_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn validate_symbol_query(query: &str) -> Result<String, DocsRsUseCaseError> {
    if query.len() > MAX_SYMBOL_QUERY_LEN {
        return Err(DocsRsUseCaseError::InvalidInput(format!(
            "query longer than {MAX_SYMBOL_QUERY_LEN} characters"
        )));
    }
    if query.chars().any(|c| c.is_control()) {
        return Err(DocsRsUseCaseError::InvalidInput(
            "query must not contain control characters".into(),
        ));
    }
    Ok(query.to_string())
}

fn match_filters(entry: &SymbolEntry, query: Option<&str>, kinds: Option<&[String]>) -> bool {
    if let Some(kinds) = kinds
        && !kinds.iter().any(|k| k == &entry.kind)
    {
        return false;
    }
    if let Some(q) = query {
        // Both sides are ASCII in any realistic case (rustdoc items),
        // but `to_lowercase` is the standards-compliant Unicode form.
        // Cost is negligible compared to the network fetch above.
        let name_l = entry.name.to_lowercase();
        let q_l = q.to_lowercase();
        if !name_l.contains(&q_l) {
            return false;
        }
    }
    true
}

/// Parse a rustdoc `all.html` page into a flat list of `SymbolEntry`.
///
/// The page has a uniform structure: one `<h3 id="kind">` per item
/// kind, followed by `<ul class="all-items">` of
/// `<li><a href="path-to-item.html">qualified::name</a></li>` rows.
/// rustdoc has emitted this shape consistently for years, so a
/// hand-rolled string walker is robust enough — no HTML parser dep
/// required.
///
/// On malformed or unexpected input the walker stops at the first
/// missing delimiter and returns whatever it parsed so far. We'd
/// rather hand back a partial list than fail the whole tool call.
fn parse_all_html(html: &str) -> Vec<SymbolEntry> {
    // Scope to <main> so we never pick up sidebar/footer link lists,
    // which would otherwise pollute the symbol index.
    let scope = extract_main_content(html);
    let mut entries = Vec::new();

    // Sections are introduced by `<h3 ` — split on that marker and
    // process each section chunk independently. Within each chunk we
    // hunt for `id="..."` and the items, both defensively: rustdoc
    // could plausibly reorder or add HTML attributes, and we'd rather
    // keep working than silently drop sections.
    for chunk in scope.split("<h3 ").skip(1) {
        let Some(tag_end) = chunk.find('>') else {
            continue;
        };
        let h3_attrs = &chunk[..tag_end];
        let Some(rustdoc_kind) = find_attr(h3_attrs, "id") else {
            continue;
        };
        let kind = normalise_kind(rustdoc_kind);

        // Body starts after the `<h3 …>` close. Item links use
        // `<li>` wrapping `<a href="…">`; we tolerate extra attributes
        // on either by hunting for `<li>` then the next `<a` after it.
        let body = &chunk[tag_end + 1..];
        const LI_OPEN: &str = "<li>";
        const CLOSE_TAG: &str = "</a></li>";
        let mut cursor = 0;
        while let Some(li_rel) = body[cursor..].find(LI_OPEN) {
            let after_li = cursor + li_rel + LI_OPEN.len();
            // Find the first `<a ` (or `<a>`) after the `<li>`. If
            // there isn't one before the next `<li>`, skip this row.
            let next_li_abs = body[after_li..]
                .find(LI_OPEN)
                .map(|r| after_li + r)
                .unwrap_or(body.len());
            let li_inner = &body[after_li..next_li_abs];
            let Some(a_rel) = li_inner.find("<a") else {
                cursor = next_li_abs;
                continue;
            };
            let after_a = after_li + a_rel + 2; // past `<a`
            let a_tag_slice = &body[after_a..];
            let Some(a_tag_end) = a_tag_slice.find('>') else {
                break;
            };
            let a_attrs = &a_tag_slice[..a_tag_end];
            let Some(path) = find_attr(a_attrs, "href") else {
                cursor = next_li_abs;
                continue;
            };
            let after_open_tag = after_a + a_tag_end + 1;
            let after_open = &body[after_open_tag..];
            let Some(close_rel) = after_open.find(CLOSE_TAG) else {
                break;
            };
            let raw_name = &after_open[..close_rel];
            // rustdoc historically wraps the link text in plain text,
            // but newer versions may wrap it in `<code>` or `<span>`.
            // Strip any tags so substring matching against the
            // caller's query stays meaningful.
            let name = strip_tags(raw_name);

            entries.push(SymbolEntry {
                kind: kind.to_string(),
                name,
                path: path.to_string(),
            });

            cursor = after_open_tag + close_rel + CLOSE_TAG.len();
        }
    }
    entries
}

/// Find `name="value"` in an HTML open-tag attribute span. Returns
/// the value if present; otherwise `None`. Whitespace before the
/// attribute is implied by the way we slice (the leading space of
/// `<h3 id=...>` is consumed by the `<h3 ` split). The match also
/// allows other characters preceding the attribute (e.g.
/// `class="x" id="y"`) by searching for ` name="` first.
fn find_attr<'a>(open_tag_attrs: &'a str, name: &str) -> Option<&'a str> {
    // Try `name="` at the very start, then ` name="` anywhere else.
    // This handles both `id="…"` (start of attrs) and `class="…" id="…"`.
    let lead = format!("{name}=\"");
    let mid = format!(" {name}=\"");
    let start = if let Some(s) = open_tag_attrs.strip_prefix(&lead) {
        s
    } else {
        let mid_pos = open_tag_attrs.find(mid.as_str())?;
        &open_tag_attrs[mid_pos + mid.len()..]
    };
    let end = start.find('"')?;
    Some(&start[..end])
}

/// Strip HTML tags from a short fragment of inline content, leaving
/// only the text. Used to clean up rustdoc link bodies that may wrap
/// the visible name in `<code>` or `<span class=…>`. Not a general
/// HTML sanitiser — just a cheap "drop everything between `<` and
/// the matching `>`" pass.
fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Map rustdoc's plural section ids to the singular kinds used in
/// the public DTO and `kinds` filter. Unknown ids pass through
/// verbatim so future rustdoc additions still surface.
fn normalise_kind(rustdoc_id: &str) -> &str {
    match rustdoc_id {
        "structs" => "struct",
        "enums" => "enum",
        "traits" => "trait",
        "unions" => "union",
        "macros" => "macro",
        "derives" => "derive",
        "attributes" => "attribute",
        "functions" => "fn",
        "types" => "type",
        "modules" => "module",
        "constants" => "constant",
        "statics" => "static",
        "primitives" => "primitive",
        other => other,
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

    /// Mini all.html fixture — same shape as real rustdoc output,
    /// just trimmed down. Sidebar content outside `<main>` is included
    /// to verify it gets dropped by the scope guard.
    const ALL_HTML_FIXTURE: &str = r#"<html><body>
        <nav class="sidebar">
            <ul><li><a href="evil/struct.Bad.html">Sidebar::Bad</a></li></ul>
        </nav>
        <main><section id="main-content" class="content">
            <h1>List of all items</h1>
            <h3 id="structs">Structs</h3>
            <ul class="all-items">
                <li><a href="struct.Error.html">Error</a></li>
                <li><a href="de/struct.IgnoredAny.html">de::IgnoredAny</a></li>
                <li><a href="de/value/struct.U8Deserializer.html">de::value::U8Deserializer</a></li>
            </ul>
            <h3 id="traits">Traits</h3>
            <ul class="all-items">
                <li><a href="trait.Deserialize.html">Deserialize</a></li>
                <li><a href="ser/trait.Serializer.html">ser::Serializer</a></li>
            </ul>
            <h3 id="derives">Derive Macros</h3>
            <ul class="all-items">
                <li><a href="derive.Deserialize.html">Deserialize</a></li>
            </ul>
        </section></main>
    </body></html>"#;

    #[test]
    fn parse_all_html_extracts_kind_normalised_entries() {
        let entries = parse_all_html(ALL_HTML_FIXTURE);
        let names: Vec<_> = entries.iter().map(|e| (&e.kind[..], &e.name[..])).collect();
        assert_eq!(
            names,
            vec![
                ("struct", "Error"),
                ("struct", "de::IgnoredAny"),
                ("struct", "de::value::U8Deserializer"),
                ("trait", "Deserialize"),
                ("trait", "ser::Serializer"),
                ("derive", "Deserialize"),
            ],
        );
        assert!(
            entries.iter().all(|e| e.name != "Sidebar::Bad"),
            "sidebar entry leaked: {entries:?}",
        );
    }

    #[test]
    fn parse_all_html_returns_paths_verbatim() {
        let entries = parse_all_html(ALL_HTML_FIXTURE);
        let item = entries
            .iter()
            .find(|e| e.name == "de::value::U8Deserializer")
            .expect("U8Deserializer in fixture");
        assert_eq!(item.path, "de/value/struct.U8Deserializer.html");
    }

    #[test]
    fn parse_all_html_tolerates_unknown_kind() {
        // Future rustdoc could add a section we don't know about.
        // Unknown kinds should pass through verbatim so callers can
        // still find the items rather than have them silently dropped.
        let html = r#"<main>
            <h3 id="lifetimes">Lifetimes</h3>
            <ul><li><a href="lt.something.html">'a</a></li></ul>
        </main>"#;
        let entries = parse_all_html(html);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "lifetimes");
    }

    #[test]
    fn parse_all_html_returns_empty_for_non_rustdoc_page() {
        let html = "<html><body><p>Sorry, we couldn't find that.</p></body></html>";
        assert!(parse_all_html(html).is_empty());
    }

    #[tokio::test]
    async fn search_symbols_filters_by_substring_case_insensitive() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/serde/1.0.200/serde/all.html".into(),
            html: ALL_HTML_FIXTURE.into(),
        }))
        .await;
        let use_case = use_case_with(stub.clone());

        let out = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "serde".into(),
                version: None,
                query: Some("desER".into()),
                kinds: None,
                limit: None,
            })
            .await?;

        let matched_names: Vec<_> = out.items.iter().map(|i| i.name.clone()).collect();
        assert!(
            matched_names.iter().any(|n| n == "Deserialize"),
            "missing Deserialize: {matched_names:?}",
        );
        assert!(
            matched_names
                .iter()
                .any(|n| n == "de::value::U8Deserializer"),
            "missing U8Deserializer: {matched_names:?}",
        );
        assert_eq!(out.resolved_version.as_deref(), Some("1.0.200"));
        assert_eq!(
            stub.last_seen_url().await.as_deref(),
            Some("https://docs.rs/serde/latest/serde/all.html"),
        );
        Ok(())
    }

    #[tokio::test]
    async fn search_symbols_respects_kind_filter() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/serde/1.0.200/serde/all.html".into(),
            html: ALL_HTML_FIXTURE.into(),
        }))
        .await;
        let use_case = use_case_with(stub);

        let out = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "serde".into(),
                version: None,
                query: Some("deserialize".into()),
                kinds: Some(vec!["TRAIT".into()]),
                limit: None,
            })
            .await?;

        assert_eq!(out.total_matched, 1);
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].kind, "trait");
        assert_eq!(out.items[0].name, "Deserialize");
        Ok(())
    }

    #[tokio::test]
    async fn search_symbols_returns_all_with_empty_query() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/serde/1.0.200/serde/all.html".into(),
            html: ALL_HTML_FIXTURE.into(),
        }))
        .await;
        let use_case = use_case_with(stub);

        let out = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "serde".into(),
                version: None,
                query: Some("   ".into()),
                kinds: None,
                limit: None,
            })
            .await?;

        assert_eq!(out.total_matched, 6);
        assert!(!out.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn search_symbols_truncates_to_limit_and_reports_total() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/serde/1.0.200/serde/all.html".into(),
            html: ALL_HTML_FIXTURE.into(),
        }))
        .await;
        let use_case = use_case_with(stub);

        let out = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "serde".into(),
                version: None,
                query: None,
                kinds: None,
                limit: Some(2),
            })
            .await?;

        assert_eq!(out.items.len(), 2);
        assert_eq!(out.total_matched, 6);
        assert!(out.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn search_symbols_rejects_overlong_query() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);

        let err = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "serde".into(),
                version: None,
                query: Some("x".repeat(129)),
                kinds: None,
                limit: None,
            })
            .await
            .expect_err("expected validation error");
        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn search_symbols_treats_empty_kinds_array_as_no_filter() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/serde/1.0.200/serde/all.html".into(),
            html: ALL_HTML_FIXTURE.into(),
        }))
        .await;
        let use_case = use_case_with(stub);

        let out = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "serde".into(),
                version: None,
                query: None,
                kinds: Some(vec![]),
                limit: None,
            })
            .await?;

        // 6 entries in the fixture; an empty kinds array means "no
        // filter", not "match nothing".
        assert_eq!(out.total_matched, 6);
        Ok(())
    }

    #[test]
    fn parse_all_html_handles_reordered_h3_attributes() {
        // rustdoc could plausibly add a class before the id; our
        // walker should still find the kind via the attribute hunt.
        let html = r#"<main>
            <h3 class="section-header" id="traits">Traits</h3>
            <ul><li><a class="trait" href="trait.Foo.html">Foo</a></li></ul>
        </main>"#;
        let entries = parse_all_html(html);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "trait");
        assert_eq!(entries[0].name, "Foo");
        assert_eq!(entries[0].path, "trait.Foo.html");
    }

    #[test]
    fn parse_all_html_strips_inner_markup_from_names() {
        // Newer rustdoc versions wrap names in <code>. Without
        // stripping, substring matching against the user's query
        // would silently break.
        let html = r#"<main>
            <h3 id="structs">Structs</h3>
            <ul><li><a href="struct.Foo.html"><code>de::Foo</code></a></li></ul>
        </main>"#;
        let entries = parse_all_html(html);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "de::Foo");
    }

    #[test]
    fn build_rustdoc_json_url_uses_crate_endpoint() {
        let u = build_rustdoc_json_url(BASE_URL, "tokio-util", "0.7.10");
        // Note the `/crate/` prefix — this is the docs.rs metadata
        // endpoint, NOT the lib-name path used by `fetch_crate_docs`.
        // No hyphen→underscore translation either.
        assert_eq!(u, "https://docs.rs/crate/tokio-util/0.7.10/json.zst");
    }

    #[test]
    fn parse_rustdoc_json_version_strips_redirect_resolution() {
        let v = parse_rustdoc_json_version(
            BASE_URL,
            "serde",
            "https://docs.rs/crate/serde/1.0.219/json.zst",
        );
        assert_eq!(v.as_deref(), Some("1.0.219"));
    }

    #[test]
    fn parse_rustdoc_json_version_returns_none_for_unexpected_shape() {
        let v = parse_rustdoc_json_version(BASE_URL, "serde", "https://docs.rs/about");
        assert_eq!(v, None);
    }

    #[test]
    fn validate_grep_query_rejects_empty_and_overlong() {
        assert!(validate_grep_query("").is_err());
        assert!(validate_grep_query(&"x".repeat(MAX_GREP_QUERY_LEN + 1)).is_err());
        assert!(validate_grep_query("pin").is_ok());
    }

    #[test]
    fn count_substr_counts_non_overlapping() {
        assert_eq!(count_substr("zero-copy zero-copy", "zero"), 2);
        assert_eq!(count_substr("aaaa", "aa"), 2);
        assert_eq!(count_substr("xyz", "w"), 0);
        assert_eq!(count_substr("anything", ""), 0);
    }

    #[test]
    fn snippet_around_centers_match_and_marks_truncation() {
        let docs = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do \
                    eiusmod tempor incididunt ut labore et zero-copy magna aliqua. \
                    Ut enim ad minim veniam, quis nostrud exercitation.";
        let docs_lower = docs.to_lowercase();
        let snippet = snippet_around(docs, &docs_lower, "zero-copy", 60);
        assert!(
            snippet.contains("zero-copy"),
            "snippet missing match: {snippet}"
        );
        assert!(
            snippet.starts_with('…') && snippet.ends_with('…'),
            "expected truncation markers on both sides: {snippet}",
        );
    }

    #[test]
    fn snippet_around_skips_truncation_marker_for_full_body() {
        let docs = "Pin is great.";
        let docs_lower = docs.to_lowercase();
        let snippet = snippet_around(docs, &docs_lower, "pin", 200);
        assert_eq!(snippet, "Pin is great.");
    }

    #[test]
    fn snippet_around_collapses_internal_newlines() {
        let docs = "First line.\n\nSecond line mentions Pin.\nThird line.";
        let docs_lower = docs.to_lowercase();
        let snippet = snippet_around(docs, &docs_lower, "pin", 200);
        assert!(!snippet.contains('\n'), "newlines leaked: {snippet:?}");
        assert!(snippet.contains("Pin"));
    }

    #[test]
    fn snippet_around_respects_utf8_boundaries() {
        // 😺 is 4 bytes; centering the snippet anywhere mid-emoji must
        // not panic and must not produce invalid UTF-8.
        let docs = "intro 😺😺😺 middle pin 😺😺😺 outro";
        let docs_lower = docs.to_lowercase();
        // Tiny target so the slice straddles the emoji clusters.
        let snippet = snippet_around(docs, &docs_lower, "pin", 8);
        assert!(snippet.contains("pin"));
        // No panic == passing. Sanity: result must round-trip as UTF-8
        // (which it does by construction, but check anyway).
        assert!(snippet.is_char_boundary(0) && snippet.is_char_boundary(snippet.len()));
    }

    #[test]
    fn snippet_around_keeps_match_when_lowercasing_changes_byte_length() {
        // Rust's default Unicode case-mapping lowercases `İ` (U+0130,
        // 2 bytes) to `i\u{307}` (3 bytes), so `docs_lower` is *longer*
        // than `docs`. The byte index returned by
        // `docs_lower.find(needle_lower)` therefore does NOT correspond
        // to the same character in the original `docs`. Centering the
        // snippet on that index drifts past the actual match — with a
        // small window the resulting snippet can miss the match entirely.
        let docs = "İİİİİPinXXXXX";
        let docs_lower = docs.to_lowercase();
        // Sanity-check the precondition that drives the bug: lowercasing
        // must have stretched the string. If a future stdlib change made
        // this no longer true the test would be vacuous, not silently
        // passing.
        assert!(
            docs_lower.len() > docs.len(),
            "test premise broken: docs_lower must be longer than docs",
        );
        let snippet = snippet_around(docs, &docs_lower, "pin", 6);
        assert!(
            snippet.to_lowercase().contains("pin"),
            "snippet must contain the match (case-insensitive); got {snippet:?}",
        );
    }

    #[test]
    fn rustdoc_kind_and_path_maps_attribute() {
        // `ItemKind::Attribute` is core's built-in attribute documentation
        // (e.g. `#[no_mangle]`, `#[repr]`); rustdoc emits an
        // `attr.{name}.html` page for it, the same shape as `ProcAttribute`.
        // Currently this kind falls through to the catch-all `_ => None`,
        // so built-in attribute items are silently dropped from grep
        // results even though they're addressable pages.
        let summary = rustdoc_types::ItemSummary {
            crate_id: 0,
            path: vec!["core".into(), "no_mangle".into()],
            kind: rustdoc_types::ItemKind::Attribute,
        };
        let (kind, path) =
            rustdoc_kind_and_path(&summary).expect("Attribute must map to a doc page");
        assert_eq!(kind, "attribute");
        assert_eq!(path, "attr.no_mangle.html");
    }

    #[test]
    fn rustdoc_kind_and_path_maps_struct() {
        let summary = rustdoc_types::ItemSummary {
            crate_id: 0,
            path: vec![
                "serde".into(),
                "de".into(),
                "value".into(),
                "U8Deserializer".into(),
            ],
            kind: rustdoc_types::ItemKind::Struct,
        };
        let (kind, path) = rustdoc_kind_and_path(&summary).expect("addressable");
        assert_eq!(kind, "struct");
        assert_eq!(path, "de/value/struct.U8Deserializer.html");
    }

    #[test]
    fn rustdoc_kind_and_path_maps_module_to_index_html() {
        let summary = rustdoc_types::ItemSummary {
            crate_id: 0,
            path: vec!["serde".into(), "de".into(), "value".into()],
            kind: rustdoc_types::ItemKind::Module,
        };
        let (kind, path) = rustdoc_kind_and_path(&summary).expect("addressable");
        assert_eq!(kind, "module");
        assert_eq!(path, "de/value/index.html");
    }

    #[test]
    fn rustdoc_kind_and_path_maps_trait_at_crate_root() {
        let summary = rustdoc_types::ItemSummary {
            crate_id: 0,
            path: vec!["serde".into(), "Deserialize".into()],
            kind: rustdoc_types::ItemKind::Trait,
        };
        let (kind, path) = rustdoc_kind_and_path(&summary).expect("addressable");
        assert_eq!(kind, "trait");
        assert_eq!(path, "trait.Deserialize.html");
    }

    #[test]
    fn rustdoc_kind_and_path_skips_crate_root() {
        let summary = rustdoc_types::ItemSummary {
            crate_id: 0,
            path: vec!["serde".into()],
            kind: rustdoc_types::ItemKind::Module,
        };
        assert!(rustdoc_kind_and_path(&summary).is_none());
    }

    #[test]
    fn rustdoc_kind_and_path_skips_non_addressable_kinds() {
        for kind in [
            rustdoc_types::ItemKind::Impl,
            rustdoc_types::ItemKind::StructField,
            rustdoc_types::ItemKind::Variant,
            rustdoc_types::ItemKind::AssocConst,
            rustdoc_types::ItemKind::AssocType,
        ] {
            let summary = rustdoc_types::ItemSummary {
                crate_id: 0,
                path: vec!["serde".into(), "Foo".into()],
                kind,
            };
            assert!(rustdoc_kind_and_path(&summary).is_none());
        }
    }

    #[test]
    fn qualified_name_strips_crate_lib_segment() {
        let summary = rustdoc_types::ItemSummary {
            crate_id: 0,
            path: vec![
                "serde".into(),
                "de".into(),
                "value".into(),
                "U8Deserializer".into(),
            ],
            kind: rustdoc_types::ItemKind::Struct,
        };
        assert_eq!(
            qualified_name_from_summary(&summary),
            "de::value::U8Deserializer",
        );
    }

    #[tokio::test]
    async fn grep_rejects_empty_query() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub);
        let err = use_case
            .grep_crate_docs(GrepCrateDocsUseCaseInput {
                crate_name: "anyhow".into(),
                version: None,
                query: "   ".into(),
                kinds: None,
                limit: None,
            })
            .await
            .expect_err("expected InvalidInput");
        assert!(matches!(err, DocsRsUseCaseError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn grep_targets_crate_json_endpoint() {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let use_case = use_case_with(stub.clone());
        // No JSON enqueued; stub returns NotFound for us. We only care
        // here that the URL the use case built was the rustdoc-JSON
        // endpoint, not the lib-name path.
        let _ = use_case
            .grep_crate_docs(GrepCrateDocsUseCaseInput {
                crate_name: "tokio-util".into(),
                version: Some("0.7.10".into()),
                query: "pin".into(),
                kinds: None,
                limit: None,
            })
            .await;
        assert_eq!(
            stub.last_seen_url().await.as_deref(),
            Some("https://docs.rs/crate/tokio-util/0.7.10/json.zst"),
        );
    }

    /// End-to-end use-case test against the real anyhow rustdoc-JSON
    /// fixture (decompressed in-process via ruzstd). Complements the
    /// transport-level test in `tests/grep_crate_docs.rs` by exercising
    /// ranking, snippet generation, and the kind filter without
    /// spinning up an MCP server or wiremock.
    #[tokio::test]
    async fn grep_against_anyhow_fixture_returns_ranked_hits() -> anyhow::Result<()> {
        use std::io::Read;
        // Path is relative to this file. The integration test in
        // `tests/grep_crate_docs.rs` uses the same fixture via a
        // shorter relative path.
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/anyhow_rustdoc.json.zst");
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(FIXTURE)?;
        let mut decompressed = Vec::with_capacity(512 * 1024);
        decoder.read_to_end(&mut decompressed)?;
        let crate_json: Arc<rustdoc_types::Crate> =
            Arc::new(serde_json::from_slice(&decompressed)?);

        let stub = Arc::new(DocsRsRepositoryStub::new());
        stub.enqueue_json(Ok(FetchRustdocJsonRepositoryOutput {
            final_url: "https://docs.rs/crate/anyhow/1.0.86/json.zst".into(),
            crate_json,
        }))
        .await;
        let use_case = use_case_with(stub);

        let out = use_case
            .grep_crate_docs(GrepCrateDocsUseCaseInput {
                crate_name: "anyhow".into(),
                version: None,
                query: "error".into(),
                kinds: None,
                limit: Some(5),
            })
            .await?;

        assert_eq!(out.crate_name, "anyhow");
        assert_eq!(out.resolved_version.as_deref(), Some("1.0.86"));
        assert!(out.total_matched > 0, "expected hits in anyhow for `error`");
        assert!(out.items.len() <= 5);
        // Every hit must carry the full quartet, and the snippet must
        // contain the query (case-insensitive) — that's the contract
        // the tool layer depends on.
        for hit in &out.items {
            assert!(!hit.kind.is_empty());
            assert!(!hit.name.is_empty());
            assert!(!hit.path.is_empty());
            assert!(
                hit.snippet.to_lowercase().contains("error"),
                "snippet missing query: {:?}",
                hit.snippet,
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn search_symbols_clamps_limit_above_max() -> anyhow::Result<()> {
        let stub = Arc::new(DocsRsRepositoryStub::new());
        let mut huge = String::from(r#"<main><h3 id="structs">Structs</h3><ul>"#);
        for i in 0..600 {
            huge.push_str(&format!("<li><a href=\"struct.S{i}.html\">S{i}</a></li>"));
        }
        huge.push_str("</ul></main>");
        stub.enqueue(Ok(FetchCrateDocsRepositoryOutput {
            final_url: "https://docs.rs/x/1.0.0/x/all.html".into(),
            html: huge,
        }))
        .await;
        let use_case = use_case_with(stub);

        let out = use_case
            .search_crate_symbols(SearchCrateSymbolsUseCaseInput {
                crate_name: "x".into(),
                version: None,
                query: None,
                kinds: None,
                limit: Some(10_000),
            })
            .await?;

        assert_eq!(
            out.items.len(),
            500,
            "limit should clamp to MAX_SYMBOL_LIMIT"
        );
        assert_eq!(out.total_matched, 600);
        Ok(())
    }
}
