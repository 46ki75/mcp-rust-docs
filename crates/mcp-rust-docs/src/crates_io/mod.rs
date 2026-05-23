//! crates.io integration, split into the three standard layers.
//!
//! [`repository`] owns the HTTP I/O against the registry, [`use_case`]
//! applies validation and clamping, and [`tool`] adapts the use case
//! to MCP tool calls with JSON Schema'd DTOs.

/// Repository layer: HTTP client against the crates.io v1 API, plus a
/// `cfg(test)`-only stub used by unit tests of the layers above.
pub mod repository;

/// Tool layer: MCP tool definitions and request/response DTOs that
/// adapt the use case onto JSON-RPC calls.
pub mod tool;

/// Use case layer: query validation, ceiling enforcement, and the
/// transformation from raw repository records into domain summaries.
pub mod use_case;
