//! The reversible compression step of the proxy pipeline.
//!
//! The rules that keep the prompt cache and preserved thinking consistent:
//!
//! 1. A text tool result is replaced only the **first** time it is sent to the model, i.e. when
//!    it is in the last message of the request (a user message).
//! 2. From then on the exact same bytes are sent on every turn of that conversation: the map
//!    `(conversation, hash of the original) -> replacement` is applied to every message, and when
//!    it disagrees with a fresh computation, the map wins.
//! 3. A result the model already saw unreplaced (an older message whose hash is not in the map,
//!    or the same text appearing unreplaced earlier in the history) is never touched.
//!
//! The conversation is the session for the main loop and `<session>/<agent id>` for a subagent:
//! both send the same session id, but each has its own history.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokred_core::Timestamp;
use tokred_core::cache::parse_duration;
use tokred_core::compression::{
    CompressedBlock, SHADOW_KIND_COMPRESSION, ShadowRecord, StoredResult,
};
use tokred_core::config::{CompressionConfig, ShadowConfig, TrafficMode};
use tokred_core::proxy::{
    HookFailure, ProxyHook, ProxyRecord, RecordSink, RequestCtx, RequestMeta,
};

use crate::preview::{PreviewRules, Replacement, estimated_tokens, retrieve_id};
use crate::scan::{Scan, scan};

/// Conversations kept in memory; beyond this the least recently used one is dropped (and
/// reloaded through the loader, if there is one, when it comes back).
pub const MAX_CONVERSATIONS: usize = 512;

/// Retention used when `[compression] retention` cannot be read.
pub const DEFAULT_RETENTION_SECS: i64 = 7 * 86_400;

/// Loads the stored replacements of one conversation (see [`CompressionHook::with_loader`]).
pub type BlockLoader = Box<dyn Fn(&str) -> Vec<CompressedBlock> + Send + Sync>;

/// The conversation a request belongs to: its session id, plus `/<agent id>` for a subagent.
/// `None` without a session id: such requests are never compressed or measured.
#[must_use]
pub fn conversation_key(meta: &RequestMeta) -> Option<String> {
    let session = meta.session_id.as_deref().filter(|s| !s.is_empty())?;
    Some(match meta.agent_id.as_deref() {
        Some(agent) => format!("{session}/{agent}"),
        None => session.to_owned(),
    })
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[derive(Debug, Default)]
struct Conversation {
    /// Hash of the original -> the bytes sent instead (enforce).
    replacements: HashMap<String, Arc<str>>,
    /// Hashes already measured (observe), so each block is recorded once, not every turn.
    shadowed: HashSet<String>,
    /// Tools whose results are no longer replaced (too many retrievals).
    excluded: HashSet<String>,
    /// The loader already ran for it.
    loaded: bool,
    /// The upstream rejected a request this hook changed and the original went instead: the
    /// model has now seen the originals, so nothing is replaced in this conversation any more.
    disabled: bool,
    last_used: u64,
}

#[derive(Debug, Default)]
struct Conversations {
    map: HashMap<String, Conversation>,
    clock: u64,
}

impl Conversations {
    /// The conversation `key`, created (and loaded once, when a loader is given) if needed.
    fn get(&mut self, key: &str, loader: Option<&BlockLoader>) -> &mut Conversation {
        if !self.map.contains_key(key) && self.map.len() >= MAX_CONVERSATIONS {
            let oldest = self
                .map
                .iter()
                .min_by_key(|(_, c)| c.last_used)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.map.remove(&oldest);
            }
        }
        self.clock += 1;
        let conversation = self.map.entry(key.to_owned()).or_default();
        conversation.last_used = self.clock;
        if let Some(loader) = loader.filter(|_| !conversation.loaded) {
            for block in loader(key) {
                conversation
                    .replacements
                    .entry(block.original_hash)
                    .or_insert_with(|| block.replacement.into());
            }
            conversation.loaded = true;
        }
        conversation
    }
}

#[derive(Debug, Default)]
struct State {
    conversations: Conversations,
    /// Tools excluded everywhere (fed by the CLI).
    excluded: HashSet<String>,
    /// A changed request was rejected: no new block is replaced anywhere (circuit breaker).
    tripped: bool,
}

/// What to do to one request, decided under the lock and applied after it.
#[derive(Debug, Default)]
struct Plan {
    /// Unit index -> text to put in its place.
    replace: Vec<(usize, Arc<str>)>,
    /// Units replaced for the first time (enforce): store the original and the bytes sent.
    new: Vec<usize>,
    /// Units measured for the first time (observe).
    shadow: Vec<usize>,
}

/// The reversible compression hook. In `observe` (the default) it only records what it would
/// save and never changes a byte; in `enforce`, and only once the `tokred_retrieve` MCP server
/// is registered, it replaces large text tool results with a preview plus an id.
pub struct CompressionHook {
    mode: TrafficMode,
    mcp_registered: bool,
    shadow: bool,
    rules: PreviewRules,
    rules_version: String,
    retention_secs: i64,
    max_retrievals: usize,
    sink: Arc<dyn RecordSink>,
    loader: Option<BlockLoader>,
    state: Mutex<State>,
}

impl std::fmt::Debug for CompressionHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressionHook")
            .field("mode", &self.effective_mode())
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

impl CompressionHook {
    /// A hook for `[compression]` and `[shadow]`. `mcp_registered` says whether the
    /// `tokred_retrieve` server is registered with Claude Code: without it `enforce` behaves as
    /// `observe`, because a replaced block could not be read back.
    #[must_use]
    pub fn new(
        config: &CompressionConfig,
        shadow: &ShadowConfig,
        mcp_registered: bool,
        sink: Arc<dyn RecordSink>,
    ) -> Self {
        let rules = PreviewRules::from_config(config);
        Self {
            mode: config.mode,
            mcp_registered,
            shadow: shadow.compression,
            rules_version: rules.rules_version(),
            rules,
            retention_secs: parse_duration(&config.retention).unwrap_or(DEFAULT_RETENTION_SECS),
            max_retrievals: usize::try_from(config.max_retrievals).unwrap_or(usize::MAX),
            sink,
            loader: None,
            state: Mutex::new(State::default()),
        }
    }

    /// Loads a conversation's stored replacements the first time it is seen in `enforce` (for
    /// example `Store::compressed_blocks`). It runs once per conversation while it stays in
    /// memory, so a session resumed days later still gets the bytes it was sent before.
    #[must_use]
    pub fn with_loader(mut self, loader: BlockLoader) -> Self {
        self.loader = Some(loader);
        self
    }

    /// The mode in effect: `enforce` only when configured **and** the MCP server is registered.
    #[must_use]
    pub fn effective_mode(&self) -> TrafficMode {
        match self.mode {
            TrafficMode::Enforce if !self.mcp_registered => TrafficMode::Observe,
            mode => mode,
        }
    }

    /// Adds stored replacements (for example the recent ones at startup). An entry already in
    /// memory is kept: the bytes first sent win.
    pub fn seed(&self, blocks: impl IntoIterator<Item = CompressedBlock>) {
        let mut state = self.lock();
        for block in blocks {
            state
                .conversations
                .get(&block.session_id, None)
                .replacements
                .entry(block.original_hash)
                .or_insert_with(|| block.replacement.into());
        }
    }

    /// Never replaces new results of these tools again, in any conversation (for example tools
    /// the CLI found retrieved too often). Blocks already replaced keep their replacement.
    pub fn exclude_tools(&self, tools: impl IntoIterator<Item = String>) {
        self.lock().excluded.extend(tools);
    }

    /// Whether a request rejected with `400` opened the circuit breaker: blocks already replaced keep
    /// their bytes (except in the rejected conversation), and no new block is replaced until the
    /// proxy restarts. The proxy should record why and turn compression off.
    #[must_use]
    pub fn is_tripped(&self) -> bool {
        self.lock().tripped
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // The state stays valid even if a thread panicked while holding the lock: every update
        // is a single insert. Keeping it is what keeps later turns consistent.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn plan(&self, key: &str, scan: &Scan, enforce: bool) -> Plan {
        let mut guard = self.lock();
        let State {
            conversations,
            excluded: excluded_everywhere,
            tripped,
        } = &mut *guard;
        let conversation = conversations.get(key, self.loader.as_ref().filter(|_| enforce));
        if conversation.disabled {
            return Plan::default();
        }

        // A replaced result retrieved more than `max_retrievals` times: its tool's previews are
        // too short, so new results of that tool are sent whole from now on.
        if enforce {
            for unit in &scan.units {
                let retrievals = scan.retrievals.get(&retrieve_id(&unit.hash));
                if conversation.replacements.contains_key(&unit.hash)
                    && retrievals.is_some_and(|&n| n > self.max_retrievals)
                {
                    conversation.excluded.insert(unit.tool.clone());
                }
            }
        }

        // Texts the model already saw unreplaced in an earlier message.
        let seen_whole: HashSet<&str> = scan
            .units
            .iter()
            .filter(|u| !u.in_last)
            .filter(|u| {
                if enforce {
                    !conversation.replacements.contains_key(&u.hash)
                } else {
                    !conversation.shadowed.contains(&u.hash)
                }
            })
            .map(|u| u.hash.as_str())
            .collect();

        let mut plan = Plan::default();
        for (index, unit) in scan.units.iter().enumerate() {
            if enforce && let Some(text) = conversation.replacements.get(&unit.hash) {
                plan.replace.push((index, Arc::clone(text)));
                continue;
            }
            let Some(candidate) = &unit.candidate else {
                continue;
            };
            if seen_whole.contains(unit.hash.as_str())
                || *tripped
                || excluded_everywhere.contains(&unit.tool)
                || conversation.excluded.contains(&unit.tool)
            {
                continue;
            }
            if enforce {
                let text: Arc<str> = candidate.text.as_str().into();
                conversation
                    .replacements
                    .insert(unit.hash.clone(), Arc::clone(&text));
                plan.replace.push((index, text));
                plan.new.push(index);
            } else if conversation.shadowed.insert(unit.hash.clone()) {
                plan.shadow.push(index);
            }
        }
        plan
    }
}

impl ProxyHook for CompressionHook {
    fn name(&self) -> &'static str {
        "compression"
    }

    /// The upstream rejected a request this hook changed, and the proxy sent the original: the
    /// model saw the originals of that conversation, so from now on it gets them on every turn.
    /// A `400` (the binding/thinking class of error) also opens the circuit breaker, which stops
    /// new replacements everywhere; a rate limit or an overload does not.
    fn fallback(&self, ctx: &RequestCtx<'_>, status: u16) {
        if ctx.notes.blocks_compressed == 0 {
            return;
        }
        let Some(key) = conversation_key(ctx.meta) else {
            return;
        };
        let mut state = self.lock();
        state.tripped |= status == 400;
        state.conversations.get(&key, None).disabled = true;
    }

    fn before(&self, ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure> {
        let enforce = match self.effective_mode() {
            TrafficMode::Enforce => true,
            TrafficMode::Observe if self.shadow => false,
            TrafficMode::Observe | TrafficMode::Off => return Ok(()),
        };
        let Some(key) = conversation_key(ctx.meta) else {
            return Ok(());
        };
        let scan = scan(&ctx.body, &self.rules);
        if scan.units.is_empty() {
            return Ok(());
        }
        let plan = self.plan(&key, &scan, enforce);
        let session = ctx.meta.session_id.clone().unwrap_or_default();
        let now = ctx.now;

        // The originals are read before the body changes.
        let new_units = plan.new.iter().map(|&index| &scan.units[index]);
        for (unit, candidate) in new_units.filter_map(|u| Some((u, u.candidate.as_ref()?))) {
            let content = ctx
                .body
                .pointer(&unit.slot.pointer())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            self.sink.record(ProxyRecord::Stored(Box::new(StoredResult {
                id: candidate.id.clone(),
                session_id: session.clone(),
                tool: unit.tool.clone(),
                original_hash: unit.hash.clone(),
                content,
                lines: to_u64(candidate.lines),
                bytes: to_u64(candidate.original_bytes),
                created_at: now,
                expires_at: Timestamp(now.0.saturating_add(self.retention_secs)),
            })));
            self.sink
                .record(ProxyRecord::Compressed(Box::new(CompressedBlock {
                    session_id: key.clone(),
                    original_hash: unit.hash.clone(),
                    replacement: candidate.text.clone(),
                    rules_version: self.rules_version.clone(),
                    ts: now,
                })));
        }
        for &index in &plan.shadow {
            let unit = &scan.units[index];
            let saved = unit.candidate.as_ref().map_or(0, Replacement::bytes_saved);
            self.sink.record(ProxyRecord::Shadow(Box::new(ShadowRecord {
                ts: now,
                session_id: session.clone(),
                kind: SHADOW_KIND_COMPRESSION.to_owned(),
                key: unit.tool.clone(),
                bytes_saved: to_u64(saved),
                est_tokens_saved: estimated_tokens(saved),
                rules_version: self.rules_version.clone(),
            })));
        }

        let mut replaced: u32 = 0;
        for (index, text) in plan.replace {
            if let Some(target) = ctx.body.pointer_mut(&scan.units[index].slot.pointer()) {
                *target = Value::String(text.to_string());
                replaced = replaced.saturating_add(1);
            }
        }
        if replaced > 0 {
            ctx.changed = true;
            ctx.notes.blocks_compressed = ctx.notes.blocks_compressed.saturating_add(replaced);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn block(conversation: &str, hash: &str) -> CompressedBlock {
        CompressedBlock {
            session_id: conversation.to_owned(),
            original_hash: hash.to_owned(),
            replacement: format!("replacement of {hash}"),
            ..CompressedBlock::default()
        }
    }

    struct NullSink;

    impl RecordSink for NullSink {
        fn record(&self, _: ProxyRecord) {}
    }

    #[test]
    fn the_least_recently_used_conversation_is_dropped_and_reloaded() {
        let loads = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&loads);
        let loader: BlockLoader = Box::new(move |key: &str| {
            counter.fetch_add(1, Ordering::SeqCst);
            vec![block(key, "h")]
        });
        let mut conversations = Conversations::default();
        for n in 0..MAX_CONVERSATIONS {
            conversations.get(&format!("c{n}"), Some(&loader));
        }
        // Touch c0 so that c1 becomes the oldest.
        conversations.get("c0", Some(&loader));
        assert_eq!(
            loads.load(Ordering::SeqCst),
            MAX_CONVERSATIONS,
            "loaded once each"
        );

        conversations.get("new", Some(&loader));
        assert_eq!(conversations.map.len(), MAX_CONVERSATIONS);
        assert!(!conversations.map.contains_key("c1"));
        assert!(conversations.map.contains_key("c0"));

        let back = conversations.get("c1", Some(&loader));
        assert_eq!(back.replacements["h"].as_ref(), "replacement of h");
        assert_eq!(loads.load(Ordering::SeqCst), MAX_CONVERSATIONS + 2);
    }

    #[test]
    fn seeded_conversations_are_still_completed_by_the_loader() {
        let loader: BlockLoader =
            Box::new(|key: &str| vec![block(key, "old"), block(key, "seeded")]);
        let mut conversations = Conversations::default();
        conversations
            .get("s", None)
            .replacements
            .insert("seeded".to_owned(), "kept".into());
        let conversation = conversations.get("s", Some(&loader));
        assert_eq!(
            conversation.replacements["old"].as_ref(),
            "replacement of old"
        );
        assert_eq!(
            conversation.replacements["seeded"].as_ref(),
            "kept",
            "the map wins"
        );
    }

    #[test]
    fn a_poisoned_lock_keeps_the_state() {
        let hook = CompressionHook::new(
            &CompressionConfig::default(),
            &ShadowConfig::default(),
            true,
            Arc::new(NullSink),
        );
        hook.seed([block("s", "h")]);
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _guard = hook.state.lock();
                    panic!("poison the lock");
                })
                .join();
        });
        assert!(hook.state.is_poisoned());
        hook.exclude_tools(["Read".to_owned()]);
        let state = hook.lock();
        assert!(state.excluded.contains("Read"));
        assert!(state.conversations.map["s"].replacements.contains_key("h"));
        drop(state);
        NullSink.record(ProxyRecord::Compressed(Box::default()));
    }
}
