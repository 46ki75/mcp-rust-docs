# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

An MCP (Model Context Protocol) server written in Rust on top of the
`rmcp` SDK. Currently exposes one tool, `search_crates`, that queries
crates.io. The crate ships as a single binary, `mcp-rust-docs`, with
two transport subcommands:

- `mcp-rust-docs stdio` — line-buffered JSON-RPC over stdin/stdout
- `mcp-rust-docs http` — streamable HTTP, mounted at `/mcp`

Both accept `--crates-io-base-url` (env `MCP_CRATES_IO_BASE_URL`)
to point at a wiremock fixture or registry mirror. The HTTP subcommand
accepts `--bind` (env `MCP_BIND_ADDRESS`, default `127.0.0.1:8000`).

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

## Testing model

Four physically separate test surfaces, each closing a different gap:

1. **Unit tests** — `#[cfg(test)] mod tests` inside `src/`. Currently
   only `crates_io/use_case/mod.rs`. Uses `CratesIoRepositoryStub`,
   which is `#[cfg(test)] pub(crate)` in `crates_io/repository/mod.rs`
   — invisible to release builds and integration tests.
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
