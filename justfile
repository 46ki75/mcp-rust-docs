default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

ci: fmt-check lint test

run-stdio:
    cargo run -p mcp-rust-docs --bin mcp-rust-docs-stdio

run-http:
    cargo run -p mcp-rust-docs --bin mcp-rust-docs-http
