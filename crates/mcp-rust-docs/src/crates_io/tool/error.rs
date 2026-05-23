use rmcp::model::{CallToolResult, Content};

use crate::crates_io::use_case::CratesIoUseCaseError;

#[derive(Debug, thiserror::Error)]
pub enum CratesIoToolError {
    #[error(transparent)]
    UseCase(#[from] CratesIoUseCaseError),
}

impl CratesIoToolError {
    pub fn into_tool_result(self) -> CallToolResult {
        tracing::warn!(error = ?self, "crates.io tool returned error");
        let message = match self {
            Self::UseCase(CratesIoUseCaseError::InvalidQuery(reason)) => {
                format!("Invalid request: {reason}")
            }
            Self::UseCase(CratesIoUseCaseError::Repository(err)) => {
                format!("Upstream failure: {err}")
            }
        };
        CallToolResult::error(vec![Content::text(message)])
    }
}
