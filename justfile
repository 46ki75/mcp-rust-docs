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

# Instrumented test run; produces profdata but no report
test-cov:
    cargo llvm-cov --no-report --workspace

# AI-friendly: per-file table (drop 100% files) + uncovered line numbers
coverage: test-cov
    cargo llvm-cov report --show-missing-lines --color=always 2>&1 | grep -v " 100.00%"

# Local HTML drilldown
coverage-html: test-cov
    cargo llvm-cov report --html --open

# CI / Codecov upload
coverage-ci: test-cov
    cargo llvm-cov report --lcov --output-path lcov.info

ci: fmt-check lint test

run-stdio:
    cargo run -p mcp-rust-docs --bin mcp-rust-docs-stdio

run-http:
    cargo run -p mcp-rust-docs --bin mcp-rust-docs-http
