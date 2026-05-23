# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

An MCP (Model Context Protocol) server written in Rust on top of the
`rmcp` SDK. Exposes four tools:

- `search_crates` — crates.io registry search.
- `get_crate_docs` — docs.rs HTML → Markdown for one page. On
  *crate-root* calls (no `path`), also bundles a `metadata` block
  fetched in parallel from crates.io: recent versions (capped at 20),
  the resolved version's `Cargo.toml` `[features]` map, and a
  dependency summary (per-kind counts plus the top runtime deps).
  Drill-down calls skip the metadata fetch — the agent has it from
  the root call. Metadata is *best-effort*: a crates.io blip
  populates `metadata_error` but the docs still ship. See *Composite
  tool: `get_crate_docs`* below.
- `search_crate_symbols` — name-based symbol index from rustdoc `all.html`.
- `search_crate_docs` — full-text search over doc comments via docs.rs's
  zstd-compressed rustdoc JSON (`/crate/{name}/{version}/json/{format_version}.zst`).
  Walks a fallback chain across supported `format_version`s so crates
  whose docs.rs build lags the current rustdoc schema (e.g. `serde`)
  still work; see the *Multi-version rustdoc-JSON dispatch* section
  below.

The crate ships as a single binary, `mcp-rust-docs`, with two transport
subcommands:

- `mcp-rust-docs stdio` — line-buffered JSON-RPC over stdin/stdout
- `mcp-rust-docs http` — streamable HTTP, mounted at `/mcp`

Both accept `--crates-io-base-url` (env `MCP_CRATES_IO_BASE_URL`) and
`--docs-rs-base-url` (env `MCP_DOCS_RS_BASE_URL`) to point at wiremock
fixtures or registry mirrors. The HTTP subcommand accepts `--bind` (env
`MCP_BIND_ADDRESS`, default `127.0.0.1:8000`).

## Commands

All workflows run through `just` — `cargo` is never invoked directly
in CI, scripts, or docs.

```bash
just ci            # gating check: fmt-check + clippy + hermetic tests
just test          # hermetic tests only
just fmt           # apply rustfmt
just lint          # clippy --all-targets -D warnings
just coverage      # AI-friendly per-file table + uncovered line list
just coverage-html # local HTML drilldown
just test-live     # live tier — hits real crates.io, normally skipped
just ci-live       # ci + live tests (never gates PR merges)
just run-stdio
just run-http
```

To run a single integration test file:
`cargo test --test <name>` (e.g. `cargo test --test e2e_stdio`).
To run a single test function:
`cargo test --test <name> -- <test_fn_name>`.
For unit tests inside `src/`: `cargo test --lib <path>` (e.g.
`cargo test --lib use_case::tests`).

Live tests are `#[ignore]`d by default; run them with
`cargo test -- --ignored` or `just test-live`.

## Architecture

The library follows the org-wide three-layer pattern documented in the
`development-standards` skill (repository → use case → tool), with
strict type isolation at each boundary:

```tree
crates/mcp-rust-docs/src/crates_io/
├── repository/    HTTP I/O against crates.io. Owns reqwest. Returns
│                  RepositoryCrateRecord (raw shape: both max_version
│                  and max_stable_version). Errors: Network /
│                  UpstreamStatus / InvalidResponse.
├── use_case/      Validation, defaulting, clamping, version-selection
│                  policy. Holds Arc<dyn CratesIoRepository>. Returns
│                  CrateSummary (post-policy: one `version` field).
│                  Errors: InvalidQuery / Repository.
└── tool/          MCP adapter. SearchCratesRequest (with schemars
                   JsonSchema) → use case → SearchCratesResponse.
                   CratesIoToolError::into_tool_result formats the
                   user-visible "Invalid request: ..." vs "Upstream
                   failure: ..." prefixes.
```

Each layer has its own `input.rs` / `output.rs` / `error.rs` with
explicit `From` conversions between layers. Don't shortcut by sharing
types across layers — the isolation is what lets the use case make
policy choices (e.g. preferring `max_stable_version`) without
contaminating the repository projection.

**Async traits use the boxed-future form, not `#[async_trait]`** —
see the `development-standards` skill's _Async traits with `Arc<dyn>`_
section. The `BoxFuture<'a, T>` alias lives in
`crates_io/repository/mod.rs`.

**`Server` is `#[derive(Clone)]`** so the streamable-HTTP transport
can hand out a fresh `Server` per session from its factory closure
without rebuilding state. The use case is behind an `Arc`, so cloning
is cheap.

**Cross-file tool dispatch.** Tool definitions live in
`crates_io/tool/mod.rs` via `#[tool_router(router = crates_io_tool_router, vis = "pub(crate)")]`,
while `ServerHandler` is implemented in `lib.rs` via
`#[tool_handler(router = self.tool_router)]` reading a stored
`ToolRouter<Server>` field. This is the pattern to follow when adding
a second tool module.

### docs.rs has two distinct endpoint families — don't confuse them

| Tool                                     | URL shape                                                                                                                          | Repository method    |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | -------------------- |
| `get_crate_docs`, `search_crate_symbols` | `{base}/{crate}/{version}/{lib_name}/...` (HTML, hyphen→underscore on lib_name)                                                    | `fetch_crate_docs`   |
| `search_crate_docs`                      | `{base}/crate/{crate}/{version}/json/{format_version}.zst` (zstd-compressed rustdoc JSON, NO lib_name segment, NO hyphen translation) | `fetch_rustdoc_json` |

The leading `/crate/` segment and the absence of the lib-name path are
the easy-to-miss differences. `build_url` (HTML) and
`build_rustdoc_json_url` (JSON) are kept separate for exactly this
reason — don't try to unify them.

The `/json/{format_version}` segment is required, not optional —
docs.rs's bare `/json` returns "whatever's latest" for the crate,
which has no relationship to what *this* build of `rustdoc-types` can
deserialize. Pinning the segment guarantees we either get bytes we
can parse or a clean 404 (which the use case routes into the fallback
chain below).

### Multi-version rustdoc-JSON dispatch

docs.rs serves rustdoc-JSON at multiple schema versions depending on
when each crate was last built. The ecosystem is **heterogeneous** at
any given moment: popular/recent crates sit at the current
`FORMAT_VERSION`, but lagging crates (notably `serde` as of early
2026) stay one or more versions behind until rebuilt. There's no
single version that works for all crates.

To handle both ends without forking the upstream type definitions, we
pull in **multiple `rustdoc-types` crates in parallel** via cargo's
package-rename feature. Each version owns one entry in
`SUPPORTED_FORMAT_VERSIONS` (`docs_rs/repository/mod.rs`) and one arm
in `parse_dispatch`. After deserialization, each upstream `Crate` is
translated into the normalized `DocsRsCrate` (`docs_rs/schema.rs`) so
the use case never references a specific upstream version directly.

The use case (`search_crate_docs`) then walks
`SUPPORTED_FORMAT_VERSIONS` in order, requesting each format URL until
one succeeds. `NotFound` triggers the next attempt; any other error
short-circuits the chain because retrying at a different format
wouldn't fix it. All-404 surfaces as `FormatVersionUnavailable` (use
case error) — distinct from `NotFound` so the tool layer can tell the
user "this crate exists but no compatible format build does."

**Adding a new format version** (`58`, `55`, etc.):

1. Add a renamed dep to workspace `Cargo.toml`:
   `rustdoc-types-NN = { package = "rustdoc-types", version = "=0.NN.0" }`
   and inherit into `crates/mcp-rust-docs/Cargo.toml`.
2. Add `impl_from_upstream!(rustdoc_types_NN);` at the bottom of
   `docs_rs/schema.rs`. If the new version added required fields the
   macro can't paper over, you'll need a hand-written `From` impl
   instead.
3. Add `rustdoc_types_NN::FORMAT_VERSION` to `SUPPORTED_FORMAT_VERSIONS`
   in dispatch preference order (highest first → newest) and a
   matching arm in `parse_dispatch`.
4. Add a format-NN fixture under `tests/fixtures/` (download via
   `curl https://docs.rs/crate/{crate}/latest/json/NN.zst`).
5. Add a `parse_dispatch_routes_format_NN_via_…` unit test in
   `repository/mod.rs`.
6. Update `tests/search_crate_docs.rs`: bump the wiremock `.expect(N)`
   on `search_crate_docs_reports_404_as_not_found` to reflect the new
   chain length.

When refreshing the existing fixtures
(`anyhow_rustdoc.json.zst` for format 57, `serde_rustdoc.json.zst`
for format 56), pull from `https://docs.rs/crate/{crate}/latest/json/{NN}.zst`
— both the inline unit tests (in `src/docs_rs/repository/mod.rs` and
`src/docs_rs/use_case/mod.rs`) and the integration tests share these
files via different relative paths, so the filenames must stay
stable.

### `ruzstd` (pure Rust), not `zstd` (libzstd C binding)

Chosen deliberately: `ruzstd` 0.8+ has both decoder _and_ encoder, so
the `search_crate_docs_reports_format_version_unsupported` test can
recompress a mutated fixture
without pulling in a C dep. Don't switch to the `zstd` crate without a
concrete reason — the C binding adds a build-system surface for
marginal speed gain on payloads that already decompress in single-digit
ms.

### CPU-bound work goes through `spawn_blocking`

zstd decompression + `serde_json::from_slice` of a multi-MB rustdoc
JSON would block a tokio runtime worker for tens of ms — long enough to
stall other in-flight requests on the HTTP transport. The repository
runs both inside `tokio::task::spawn_blocking`. **If you refactor
`fetch_rustdoc_json` and move that work back onto the async path, you
will silently regress concurrency under load.**

### `fetch_rustdoc_json` is cached; HTML is not

`CachingDocsRsRepository` (`docs_rs/repository/cache.rs`) is a
decorator wrapped around the real repository by `ServerBuilder` by
default. It caches successful `FetchRustdocJsonRepositoryOutput`
values in an in-process bounded TTL cache keyed by the requested URL
string (default: 16 entries, 10-minute TTL, via `moka` — W-TinyLFU
eviction, not strict LRU). Rationale:
`search_crate_docs` reliably hits the same `(crate, version)` JSON
payload across multiple queries in a session, and the per-call cost
(network + zstd + serde) dwarfs the substring walk. Errors are
intentionally NOT cached — agents retry expecting fixes, and 404s can
be transient.

HTML (`fetch_crate_docs`) is pass-through. `get_crate_docs` targets
a different page per call (low repeat rate), and `search_crate_symbols`
re-fetches the same `all.html` but pays only a network cost (no parse),
so HTML caching's value is materially lower. Revisit if `all.html`
re-fetches appear as a real cost in traces.

Disable from the CLI with `--docs-rs-cache=false` (env
`MCP_DOCS_RS_CACHE=false`). Disable in a builder via
`.docs_rs_cache_enabled(false)` — required for any wiremock-driven
integration test that asserts exact `.expect(N)` upstream call counts
against the JSON endpoint, since the cache would silently swallow
repeat fetches. `tests/search_crate_docs.rs` does this for its
existing tests and exercises the cache itself in a single dedicated
test.

### Composite tool: `get_crate_docs`

The only tool that crosses module boundaries. On a *crate-root* call
(`path` is `None`) the handler runs two use cases in parallel via
`tokio::join!`:

- `DocsRsUseCase::fetch_crate_docs` — the load-bearing docs fetch.
- `CratesIoUseCase::get_crate_metadata` — versions, features, deps.

Drill-down calls (`path` set) skip the metadata fetch entirely: the
agent already has the metadata from the root call, and re-fetching
per page would be wasted upstream calls. The `.expect(0)` mocks on
the crates.io routes in `tests/get_crate_docs.rs::get_crate_docs_omits_metadata_when_path_is_set`
pin this.

**Best-effort merging.** A metadata-only failure populates the
response's `metadata_error` (the formatted error string) and leaves
`metadata` absent. The tool result stays *success* — a crates.io
blip must not kill a successful docs fetch, which is what the agent
actually came for. A docs-fetch failure, by contrast, still surfaces
as a tool error.

**The cross-module type leak is contained.** `docs_rs/tool/response.rs`
imports `CrateMetadata` (and friends) from `crates_io/use_case` to
build its DTOs. That import is the only place outside `crates_io/`
that references those types; the use cases themselves remain
independent. If a second composite tool ever appears, consider
lifting the DTO conversion into a shared module before letting the
cross-module imports spread.

**Hermetic-test caveat.** Every test in `tests/get_crate_docs.rs`
points both `docs_rs_base_url` AND `crates_io_base_url` at the same
wiremock instance — otherwise the metadata fetch on root calls would
silently hit real crates.io. Tests that don't care about metadata
leave the `/api/v1/crates/...` routes unmocked; those requests 404,
the response carries a populated `metadata_error`, and the docs
payload still ships.

## Testing model

Four physically separate test surfaces, each closing a different gap:

1. **Unit tests** — `#[cfg(test)] mod tests` inside `src/`. Lives in
   `crates_io/use_case/mod.rs` and `docs_rs/use_case/mod.rs`. Each uses
   a sibling stub (`CratesIoRepositoryStub` / `DocsRsRepositoryStub`)
   that is `#[cfg(test)] pub(crate)` in the matching `repository/mod.rs`
   — invisible to release builds and integration tests. The docs.rs
   stub exposes paired enqueue methods (`enqueue` for HTML,
   `enqueue_json` for rustdoc JSON) so both repository trait methods
   can be exercised.
2. **`tests/search_crates.rs`** — in-process duplex pipe
   (`tokio::io::duplex`) + wiremock. Fastest; isolates protocol/handler
   logic from real I/O.
3. **`tests/e2e_http.rs`** — real TCP loopback on `127.0.0.1:0`,
   in-process axum server, real `StreamableHttpClientTransport` from
   rmcp. Proves the HTTP transport and axum mounting work.
4. **`tests/e2e_stdio.rs`** — spawns the compiled `mcp-rust-docs`
   binary as a subprocess via `env!("CARGO_BIN_EXE_mcp-rust-docs")`,
   uses `MCP_CRATES_IO_BASE_URL` to redirect to wiremock, drives it
   via rmcp's `TokioChildProcess` transport. The bytes round-trip
   through real OS pipes and the shipped binary — closest thing in CI
   to how an MCP host launches the server.
5. **`tests/live.rs`** — hits real crates.io. `#[ignore]`d with reason
   strings; tests prefixed `live_` per the standards' grep-ability rule.
   Per `development-standards`, live-test failures MUST NOT block PR
   merges (they fail for reasons unrelated to the diff).

When adding tests, decide tier first: stub-only logic → inline
`#[cfg(test)]`; transport/protocol behavior → `tests/`; upstream
contract verification → `tests/live.rs` with `#[ignore]`.

## CI

- `.github/workflows/ci.yml` — runs `just ci` on PRs and `main` pushes
- `.github/workflows/ci-live.yml` — `workflow_dispatch` + weekly cron
  (Mondays 06:00 UTC). Never gates PRs.
- `.github/workflows/publish.yml` — triggered on `v*` tag push; uses
  crates.io trusted publishing via OIDC (no long-lived token).
  Auto-creates a GitHub release with `--prerelease` flag detected from
  the semver pre-release suffix (anything after `-`, e.g.
  `1.0.0-alpha.0`). Publishes with `--locked`.
- `.github/dependabot.yml` — weekly cargo + github-actions updates,
  with minor+patch bundled into a single PR per ecosystem.

## Conventions worth knowing

- **Toolchain pinned in `rust-toolchain.toml` (1.90)**. CI installs
  exactly this via `actions-rust-lang/setup-rust-toolchain@v1`. Don't
  bump it casually — it's also the workspace's declared MSRV.
- **`Cargo.lock` is committed**. Required because this is a binary
  crate, and the `--locked` publish step depends on it.
- **`#![deny(missing_docs)]` on the library root**. Every `pub` item
  needs a doc comment, including `pub mod`. The lint won't compile
  without one.
- **Workspace inheritance is strict**. All shared deps live in root
  `Cargo.toml`'s `[workspace.dependencies]` and are pulled into member
  crates with `{ workspace = true }`. Don't pin versions in member
  crates — that's a standards violation. Single-use deps may stay
  local but check with the user first.
- **`tracing` over stdout is forbidden in stdio mode**. The MCP host
  parses every byte of stdout; the stdio entry point routes tracing
  to stderr with ANSI stripped. Don't change this without thinking
  hard about it.
