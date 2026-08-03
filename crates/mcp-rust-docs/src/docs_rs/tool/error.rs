use rmcp::model::{CallToolResult, ContentBlock};

use crate::docs_rs::repository::DocsRsRepositoryError;
use crate::docs_rs::use_case::DocsRsUseCaseError;

/// Tool-layer wrapper around use case failures, responsible for
/// rendering them as MCP [`CallToolResult::error`] payloads.
///
/// Kept distinct from `DocsRsUseCaseError` so the formatting choice
/// (what the model actually sees) lives at the protocol boundary,
/// not inside the use case.
#[derive(Debug, thiserror::Error)]
pub enum DocsRsToolError {
    /// Underlying use case failure.
    #[error(transparent)]
    UseCase(#[from] DocsRsUseCaseError),
}

impl DocsRsToolError {
    /// Convert into an error-flavored [`CallToolResult`] with a
    /// human-readable message prefix that distinguishes user errors
    /// (`Invalid request: ...`), missing pages (`Not found: ...`)
    /// from upstream failures (`Upstream failure: ...`).
    pub fn into_tool_result(self) -> CallToolResult {
        tracing::warn!(error = ?self, "docs.rs tool returned error");
        let message = match self {
            Self::UseCase(DocsRsUseCaseError::InvalidInput(reason)) => {
                format!("Invalid request: {reason}")
            }
            Self::UseCase(DocsRsUseCaseError::Repository(DocsRsRepositoryError::NotFound {
                url,
            })) => {
                format!("Not found: docs.rs returned 404 for {url}")
            }
            Self::UseCase(DocsRsUseCaseError::FormatVersionUnavailable { crate_name, tried }) => {
                format!(
                    "Not found: docs.rs has no rustdoc JSON for {crate_name} at any \
                     format version this build understands (tried {tried:?}). The crate \
                     may need to be rebuilt on docs.rs, or a newer mcp-rust-docs that \
                     supports the current format may be required."
                )
            }
            Self::UseCase(DocsRsUseCaseError::Repository(err)) => {
                format!("Upstream failure: {err}")
            }
            Self::UseCase(DocsRsUseCaseError::Internal(reason)) => {
                format!("Upstream failure: {reason}")
            }
        };
        CallToolResult::error(vec![ContentBlock::text(message)])
    }
}
