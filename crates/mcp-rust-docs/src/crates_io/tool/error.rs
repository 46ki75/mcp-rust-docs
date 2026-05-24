use rmcp::model::{CallToolResult, Content};

use crate::crates_io::use_case::CratesIoUseCaseError;

/// Tool-layer wrapper around use case failures, responsible for
/// rendering them as MCP [`CallToolResult::error`] payloads.
///
/// Kept distinct from `CratesIoUseCaseError` so the formatting choice
/// (what the model actually sees) lives at the protocol boundary,
/// not inside the use case.
#[derive(Debug, thiserror::Error)]
pub enum CratesIoToolError {
    /// Underlying use case failure.
    #[error(transparent)]
    UseCase(#[from] CratesIoUseCaseError),
}

impl CratesIoToolError {
    /// Convert into an error-flavored [`CallToolResult`] with a
    /// human-readable message prefix that distinguishes user errors
    /// (`Invalid request: ...`) from upstream failures
    /// (`Upstream failure: ...`).
    pub fn into_tool_result(self) -> CallToolResult {
        tracing::warn!(error = ?self, "crates.io tool returned error");
        let message = match self {
            Self::UseCase(CratesIoUseCaseError::InvalidQuery(reason)) => {
                format!("Invalid request: {reason}")
            }
            Self::UseCase(CratesIoUseCaseError::InconsistentUpstream(reason)) => {
                format!("Upstream failure: {reason}")
            }
            Self::UseCase(CratesIoUseCaseError::Repository(err)) => {
                format!("Upstream failure: {err}")
            }
        };
        CallToolResult::error(vec![Content::text(message)])
    }
}
