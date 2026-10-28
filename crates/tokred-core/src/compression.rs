//! Reversible compression of large tool results and shadow-mode measurements (spec 6.6, 6.7).
//! No information is lost: every replaced block is kept and can be read back with the
//! `tokred_retrieve` tool.
//!
//! This module holds the data shared with the store and the CLI; the logic lives in the
//! `tokred-compress` crate.

use serde::Serialize;

use crate::types::Timestamp;

/// [`ShadowRecord::kind`] of what reversible compression would have saved.
pub const SHADOW_KIND_COMPRESSION: &str = "compression";

/// [`ShadowRecord::kind`] of a request that exactly repeats one seen in the last 24 hours.
pub const SHADOW_KIND_RESPONSE_CACHE: &str = "response_cache";

/// An original tool result kept for `tokred_retrieve` (row of `stored_results`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StoredResult {
    /// Id given to the model (derived from the content hash).
    pub id: String,
    /// Session where it was stored.
    pub session_id: String,
    /// The tool that produced it.
    pub tool: String,
    /// blake3 of the original.
    pub original_hash: String,
    /// The original content.
    pub content: String,
    /// Lines in the original.
    pub lines: u64,
    /// Bytes of the original.
    pub bytes: u64,
    /// When it was stored.
    pub created_at: Timestamp,
    /// When it may be deleted.
    pub expires_at: Timestamp,
}

/// The exact bytes sent for a replaced block in a conversation (row of `compressed_blocks`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CompressedBlock {
    /// The conversation key: the session id for the main loop, `<session id>/<agent id>` for a
    /// subagent. Subagents share the main loop's session id but have their own history, so a
    /// block replaced in one must never change a block the other already saw.
    pub session_id: String,
    /// blake3 of the original block.
    pub original_hash: String,
    /// The replacement text sent upstream on every turn.
    pub replacement: String,
    /// Version of the compression rules that produced it.
    pub rules_version: String,
    /// When it was first sent.
    pub ts: Timestamp,
}

/// What a shadow-mode feature would have saved (row of `shadow_savings`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ShadowRecord {
    /// When.
    pub ts: Timestamp,
    /// Session id.
    pub session_id: String,
    /// `compression` or `response_cache`.
    pub kind: String,
    /// The tool (compression) or the request hash (response cache).
    pub key: String,
    /// Bytes that would not have been sent.
    pub bytes_saved: u64,
    /// Estimated tokens that would not have been sent (bytes / 4; an estimate).
    pub est_tokens_saved: u64,
    /// Version of the rules.
    pub rules_version: String,
}

/// Shadow-mode savings of one kind and key between two instants (aggregate of
/// `shadow_savings`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ShadowSavings {
    /// `compression` or `response_cache`.
    pub kind: String,
    /// The tool (compression) or the request hash (response cache).
    pub key: String,
    /// Number of records.
    pub events: u64,
    /// Number of distinct sessions.
    pub sessions: u64,
    /// Sum of the bytes that would not have been sent.
    pub bytes_saved: u64,
    /// Sum of the estimated tokens that would not have been sent.
    pub est_tokens_saved: u64,
}
