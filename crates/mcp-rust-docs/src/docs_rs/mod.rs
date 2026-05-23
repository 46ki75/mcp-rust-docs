//! docs.rs integration, split into the three standard layers.
//!
//! [`repository`] owns the HTTP I/O against docs.rs, [`use_case`]
//! validates input and converts the fetched HTML to Markdown, and
//! [`tool`] adapts the use case to MCP tool calls with JSON Schema'd
//! DTOs.

/// Repository layer: HTTP client against docs.rs, plus a `cfg(test)`-only
/// stub used by unit tests of the layers above.
pub mod repository;

/// Tool layer: MCP tool definitions and request/response DTOs that
/// adapt the use case onto JSON-RPC calls.
pub mod tool;

/// Use case layer: input validation, URL assembly, HTML-to-Markdown
/// conversion.
pub mod use_case;
