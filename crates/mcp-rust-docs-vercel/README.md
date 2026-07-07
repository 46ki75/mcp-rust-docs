# mcp-rust-docs-vercel

Vercel deployment entry point for [`mcp-rust-docs`](../mcp-rust-docs).
Not published to crates.io.

Serves the streamable-HTTP MCP transport at `/mcp` on Vercel's Rust
runtime (public beta, Fluid compute), in **stateless mode** — see the
doc comment in [`api/mcp.rs`](api/mcp.rs) for why.

## Vercel project settings

| Setting                                            | Value                                    |
| -------------------------------------------------- | ---------------------------------------- |
| Root Directory                                     | `crates/mcp-rust-docs-vercel`            |
| "Include files outside the root directory" | **On** (the crate builds via the workspace root) |

## Environment variables

All optional:

| Variable                 | Purpose                                                                                          |
| ------------------------ | ------------------------------------------------------------------------------------------------ |
| `MCP_CRATES_IO_BASE_URL` | Override the crates.io base URL (mirrors, proxies).                                              |
| `MCP_DOCS_RS_BASE_URL`   | Override the docs.rs base URL.                                                                   |
| `MCP_DOCS_RS_CACHE`      | `false` disables the in-process rustdoc-JSON cache.                                              |
| `MCP_ALLOWED_HOSTS`      | Comma-separated extra `Host` values, e.g. custom domains. Vercel's own URLs are picked up automatically from `VERCEL_URL` / `VERCEL_BRANCH_URL` / `VERCEL_PROJECT_PRODUCTION_URL`. |

## Routing

`vercel.json` rewrites every path to the single `/api/mcp` function;
the axum router inside only answers on `/mcp`. Point MCP clients at
`https://<deployment>/mcp`.
