//! Reversible compression of large tool results (spec 6.6 and 6.7) and the `tokred mcp` server
//! with the `tokred_retrieve` tool. No information is lost: every replaced text is stored and
//! can be read back.
//!
//! * [`preview`] – the pure, deterministic replacement (preview + marker) and paging;
//! * [`CompressionHook`] – the proxy step that replaces a text tool result the first time it is
//!   sent and repeats the same bytes on every later turn (`observe` only measures);
//! * [`ResponseCacheShadow`] – measures exact request repeats (shadow mode only);
//! * [`mcp`] – the stdio MCP server the CLI runs as `tokred mcp`.

pub mod hook;
pub mod mcp;
pub mod preview;
pub mod response_cache;
pub mod scan;

pub use hook::{BlockLoader, CompressionHook, conversation_key};
pub use mcp::serve;
pub use preview::{PreviewRules, Replacement, replacement};
pub use response_cache::{ResponseCacheShadow, canonical_hash};
