//! `tokred proxy start --foreground`: composes the real proxy. The proxy crate does the
//! networking; this module opens the database, builds the hook pipeline (response-cache shadow,
//! routing, reversible compression, prompt tracking) and the single writer that stores what the
//! proxy records, and keeps the pid file while it runs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tokred_compress::{CompressionHook, ResponseCacheShadow};
use tokred_core::compression::CompressedBlock;
use tokred_core::meta::MetaRepo;
use tokred_core::prompts::{PromptRepo, SuggestionRepo};
use tokred_core::routing::RoutingRepo;
use tokred_core::{Clock, Config, Timestamp};
use tokred_pricing::load_table;
use tokred_proxy::{ApplyRecord, ServerConfig, Shutdown, spawn_writer};
use tokred_routing::RoutingHook;
use tokred_store::Store;
use tokred_suggest::{PromptTracker, TrackerSettings};

use super::SystemClock;
use super::proxy::files;
use crate::env::Env;

/// How often prompts that went quiet are closed and emitted.
const FLUSH_EVERY: Duration = Duration::from_secs(30);
/// How often data older than `[proxy] retention_days` is deleted (and once at start-up).
const PRUNE_EVERY: Duration = Duration::from_secs(3600);
/// Days of compressed blocks re-read at start-up, so a restart keeps replacing the same bytes.
const SEED_DAYS: i64 = 7;

/// Whether the `tokred` MCP server is registered in any of Claude Code's user config files.
#[must_use]
pub fn mcp_registered(candidates: &[PathBuf]) -> bool {
    candidates
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .any(|config| tokred_install::mcp::is_registered(&config))
}

/// Stores one record. Each kind goes to its own table; failures are reported to the writer,
/// which counts them and never blocks a request.
pub fn apply_record(store: &Store, record: &tokred_core::proxy::ProxyRecord) -> Result<(), String> {
    use tokred_core::proxy::ProxyRecord as R;
    let failed = |err: tokred_core::StoreError| err.to_string();
    match record {
        R::Request(request) => store.insert_proxy_request(request).map(|_| ()),
        R::Decision(decision) => store.insert_decision(decision),
        R::Prompt(prompt) => store.upsert_prompt(prompt),
        R::Suggestion(suggestion) => store.insert_suggestion(suggestion),
        R::Shadow(shadow) => store
            .insert_shadow_records(std::slice::from_ref(shadow))
            .map(|_| ()),
        R::Stored(stored) => store.put_stored_result(stored),
        R::Compressed(block) => store.insert_compressed_block(block).map(|_| ()),
    }
    .map_err(failed)
}

/// Everything the proxy needs, built from the data folder.
pub struct Assembly {
    /// Proxy settings.
    pub server: ServerConfig,
    /// The hook pipeline.
    pub hooks: Vec<Arc<dyn tokred_core::proxy::ProxyHook>>,
    /// The prompt tracker (flushed on a timer and at shutdown).
    pub tracker: Arc<PromptTracker>,
    /// The compression hook (its circuit breaker is persisted).
    pub compression: Arc<CompressionHook>,
    /// The record sink, and the writer thread that stores what it receives.
    pub sink: Arc<tokred_proxy::ChannelSink>,
    /// Ends with the writer's report.
    pub writer: std::thread::JoinHandle<tokred_proxy::WriterReport>,
}

/// Builds the pipeline for `config` against the database at `db`.
pub fn assemble(
    config: &Config,
    db: &Path,
    secret: &str,
    registered: bool,
    now: Timestamp,
) -> Result<Assembly> {
    let mut server = ServerConfig::from_proxy_config(&config.proxy, secret)
        .context("invalid [proxy] settings in tokred.toml")?;
    server.routing_mode = Some(config.routing.mode.as_str());

    let writer_store =
        Store::open(db).with_context(|| format!("could not open {}", db.display()))?;
    let apply: ApplyRecord = Box::new(move |record| apply_record(&writer_store, record));
    let (sink, writer) = spawn_writer(apply);
    let sink = Arc::new(sink);

    let reader = Store::open(db).with_context(|| format!("could not open {}", db.display()))?;
    let prices = load_table(&reader, &SystemClock);
    let since = Timestamp(now.0 - SEED_DAYS * 86_400);
    let seed = reader.recent_compressed_blocks(since).unwrap_or_default();

    // The compression hook reads its replacements back through a connection of its own.
    let loader_store = Mutex::new(reader);
    let loader = Box::new(move |conversation: &str| load_blocks(&loader_store, conversation));
    let compression = CompressionHook::new(
        &config.compression,
        &config.shadow,
        registered,
        sink.clone(),
    )
    .with_loader(loader);
    compression.seed(seed);

    let tracker = Arc::new(PromptTracker::new(
        prices.clone(),
        TrackerSettings::from_config(&config.suggest),
        sink.clone(),
    ));
    let compression = Arc::new(compression);
    let hooks: Vec<Arc<dyn tokred_core::proxy::ProxyHook>> = vec![
        Arc::new(ResponseCacheShadow::new(
            config.shadow.response_cache,
            sink.clone(),
        )),
        Arc::new(RoutingHook::new(
            config.routing.clone(),
            prices,
            sink.clone(),
        )),
        compression.clone(),
        tracker.clone(),
    ];
    Ok(Assembly {
        server,
        hooks,
        tracker,
        compression,
        sink,
        writer,
    })
}

/// The compression circuit breaker survives restarts: while its mark is in the database, the
/// proxy starts with compression in `observe` until the user runs `tokred compression on`.
fn apply_persisted_breaker(config: &mut Config, db: &Path) {
    let tripped = Store::open(db)
        .ok()
        .and_then(|store| {
            store
                .meta_get(super::advice::COMPRESSION_TRIPPED_AT)
                .ok()
                .flatten()
        })
        .is_some_and(|at| !at.is_empty());
    if tripped && config.compression.mode == tokred_core::config::TrafficMode::Enforce {
        tracing::warn!("compression circuit breaker is tripped; starting in observe mode");
        config.compression.mode = tokred_core::config::TrafficMode::Observe;
    }
}

/// Runs the proxy in this process until it is stopped. Blocks.
pub fn run(env: &Env, config: &Config, claude_dir: Option<&Path>, bind: &str) -> Result<()> {
    let mut config = config.clone();
    bind.clone_into(&mut config.proxy.bind);
    apply_persisted_breaker(&mut config, &env.data.db_path());
    let proxy_files = files(env);
    let secret = proxy_files.secret()?;
    let registered = claude_dir.is_some_and(|dir| {
        mcp_registered(&tokred_audit::global_config_candidates(
            dir,
            env.home_dir.as_deref(),
        ))
    });
    let now = SystemClock.now();
    let Assembly {
        server,
        hooks,
        tracker,
        compression,
        sink,
        writer,
    } = assemble(&config, &env.data.db_path(), &secret, registered, now)?;

    let proxy = tokred_proxy::Proxy::bind(server)?;
    let port = proxy.local_addr().port();
    proxy_files.write_pid(std::process::id(), port)?;
    tracing::info!(port, registered, "tokred proxy started");

    let shutdown = Shutdown::new();
    let flusher = spawn_flusher(
        tracker.clone(),
        compression.clone(),
        shutdown.clone(),
        env.data.db_path(),
        FLUSH_EVERY,
    );

    let pruner = spawn_pruner(
        shutdown.clone(),
        env.data.db_path(),
        config.proxy.retention_days,
        PRUNE_EVERY,
    );

    let record_sink: Arc<dyn tokred_core::proxy::RecordSink> = sink.clone();
    let result = proxy.serve_blocking(hooks, record_sink, shutdown.clone());

    shutdown.trigger();
    let _ = flusher.join();
    let _ = pruner.join();
    persist_breaker(&compression, &env.data.db_path());
    tracker.flush_all(SystemClock.now());
    // Every holder of the sink must be dropped before the writer can finish: the hooks (moved
    // into the proxy), the tracker and the compression hook each keep a clone.
    drop((tracker, compression, sink));
    let _ = writer.join().map(|report| {
        tracing::info!(
            written = report.written,
            failed = report.failed,
            "tokred proxy stopped"
        );
    });
    proxy_files.clear_pid();
    result.map_err(Into::into)
}

/// The replacements stored for a conversation; none when the lookup cannot be made (the hook
/// then sends the request unchanged).
fn load_blocks(store: &Mutex<Store>, conversation: &str) -> Vec<CompressedBlock> {
    store
        .lock()
        .map(|store| store.compressed_blocks(conversation).unwrap_or_default())
        .unwrap_or_default()
}

/// Runs [`flush_loop`] on a thread of its own, saving the compression breaker with every tick.
fn spawn_flusher(
    tracker: Arc<PromptTracker>,
    compression: Arc<CompressionHook>,
    shutdown: Shutdown,
    db: PathBuf,
    every: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        flush_loop(&tracker, &shutdown, every, &|| {
            persist_breaker(&compression, &db);
        });
    })
}

/// Runs [`prune_loop`] on a thread of its own, applying `[proxy] retention_days`.
fn spawn_pruner(
    shutdown: Shutdown,
    db: PathBuf,
    retention_days: u32,
    every: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        prune_loop(&shutdown, every, &|| prune_old_data(&db, retention_days));
    })
}

/// Deletes the proxy's data older than `retention_days` (0 keeps everything). Never fails the
/// proxy: a database that cannot be opened or pruned is logged and tried again next time.
pub fn prune_old_data(db: &Path, retention_days: u32) {
    if retention_days == 0 {
        return;
    }
    let pruned = Store::open(db)
        .map_err(|err| err.to_string())
        .and_then(|store| {
            store
                .prune_proxy_data(SystemClock.now(), retention_days)
                .map_err(|err| err.to_string())
        });
    match pruned {
        Ok(report) => tracing::info!(
            rows = report.total(),
            retention_days,
            "pruned old proxy data"
        ),
        Err(error) => tracing::warn!(%error, "could not prune old proxy data"),
    }
}

/// Calls `tick` at once and then every `every` until `shutdown` is triggered.
pub fn prune_loop(shutdown: &Shutdown, every: Duration, tick: &dyn Fn()) {
    let step = Duration::from_millis(200).min(every);
    let mut waited = every;
    while !shutdown.is_triggered() {
        if waited >= every {
            waited = Duration::ZERO;
            tick();
        }
        std::thread::sleep(step);
        waited += step;
    }
}

/// Records in the database, once, that the compression circuit breaker tripped.
pub fn persist_breaker(compression: &CompressionHook, db: &Path) {
    if !compression.is_tripped() {
        return;
    }
    let Ok(store) = Store::open(db) else {
        return;
    };
    let already = store
        .meta_get(super::advice::COMPRESSION_TRIPPED_AT)
        .ok()
        .flatten()
        .is_some_and(|at| !at.is_empty());
    if !already {
        let _ = store.meta_set(
            super::advice::COMPRESSION_TRIPPED_AT,
            &SystemClock.now().0.to_string(),
        );
    }
}

/// Closes quiet prompts every `every` until `shutdown` is triggered, running `tick` each time.
pub fn flush_loop(tracker: &PromptTracker, shutdown: &Shutdown, every: Duration, tick: &dyn Fn()) {
    let step = Duration::from_millis(200).min(every);
    let mut waited = Duration::ZERO;
    while !shutdown.is_triggered() {
        std::thread::sleep(step);
        waited += step;
        if waited >= every {
            waited = Duration::ZERO;
            tracker.flush_idle(SystemClock.now());
            tick();
        }
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::compression::{ShadowRecord, StoredResult};
    use tokred_core::hook_event::HookEventRepo as _;
    use tokred_core::prompts::{PromptSummary, SuggestionRecord};
    use tokred_core::proxy::{ProxyRecord, ProxyRequestRecord};
    use tokred_core::routing::RoutingDecision;

    use super::super::testing::Sandbox;
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn every_kind_of_record_reaches_its_table() {
        let (_dir, store) = store();
        let now = Timestamp(1_800_000_000);
        let records = vec![
            ProxyRecord::Request(Box::new(ProxyRequestRecord {
                id: "r1".into(),
                ts: now,
                ..ProxyRequestRecord::default()
            })),
            ProxyRecord::Decision(Box::new(RoutingDecision {
                id: "d1".into(),
                ts: now,
                ..RoutingDecision::default()
            })),
            ProxyRecord::Prompt(Box::new(PromptSummary {
                prompt_id: "p1".into(),
                ts: now,
                ..PromptSummary::default()
            })),
            ProxyRecord::Suggestion(Box::new(SuggestionRecord {
                id: "s1".into(),
                ts: now,
                session_id: "x".into(),
                ..SuggestionRecord::default()
            })),
            ProxyRecord::Shadow(Box::new(ShadowRecord {
                ts: now,
                kind: "compression".into(),
                ..ShadowRecord::default()
            })),
            ProxyRecord::Stored(Box::new(StoredResult {
                id: "st1".into(),
                created_at: now,
                expires_at: Timestamp(now.0 + 60),
                ..StoredResult::default()
            })),
            ProxyRecord::Compressed(Box::new(CompressedBlock {
                session_id: "x".into(),
                original_hash: "h".into(),
                ts: now,
                ..CompressedBlock::default()
            })),
        ];
        for record in &records {
            apply_record(&store, record).unwrap();
        }
        assert_eq!(
            store
                .proxy_requests_between(now, Timestamp(now.0 + 1))
                .unwrap()
                .len(),
            1
        );
        assert!(store.decision("d1").unwrap().is_some());
        assert_eq!(
            store
                .prompts_between(now, Timestamp(now.0 + 1))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .suggestions_between(now, Timestamp(now.0 + 1))
                .unwrap()
                .len(),
            1
        );
        assert!(store.stored_result("st1", now).unwrap().is_some());
        assert_eq!(store.compressed_blocks("x").unwrap().len(), 1);
        assert_eq!(
            store
                .shadow_savings(now, Timestamp(now.0 + 1))
                .unwrap()
                .len(),
            1
        );
        let _ = store.events_between(now, now);
    }

    #[test]
    fn a_failing_table_is_reported_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = Store::open(&db).unwrap();
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch("DROP TABLE proxy_requests;")
            .unwrap();
        let record = ProxyRecord::Request(Box::default());
        assert!(apply_record(&store, &record).is_err());
    }

    #[test]
    fn mcp_registration_is_found_in_any_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let (none, with, junk) = (
            tmp.path().join("a.json"),
            tmp.path().join("b.json"),
            tmp.path().join("c.json"),
        );
        std::fs::write(
            &with,
            r#"{"mcpServers":{"tokred":{"type":"stdio","command":"/x/tokred","args":["mcp"]}}}"#,
        )
        .unwrap();
        std::fs::write(&junk, "{ not json").unwrap();
        let missing = tmp.path().join("missing.json");
        assert!(!mcp_registered(&[
            none.clone(),
            junk.clone(),
            missing.clone()
        ]));
        assert!(mcp_registered(&[missing, junk, with]));
        assert!(!mcp_registered(&[]));
    }

    #[test]
    fn the_pipeline_is_assembled_from_the_data_folder() {
        let sb = Sandbox::new();
        sb.configure();
        let config = Config::default();
        let assembly = assemble(
            &config,
            &sb.env.data.db_path(),
            &"s".repeat(64),
            false,
            Timestamp(1_800_000_000),
        )
        .unwrap();
        let names: Vec<_> = assembly.hooks.iter().map(|h| h.name()).collect();
        assert_eq!(names.len(), 4, "{names:?}");
        assert_eq!(assembly.server.bind.port(), 8787);
        let Assembly {
            hooks,
            tracker,
            compression,
            sink,
            writer,
            ..
        } = assembly;
        drop((hooks, tracker, compression, sink));
        let report = writer.join().unwrap();
        assert_eq!(report.written, 0);
    }

    #[test]
    fn bad_settings_and_a_missing_database_are_reported() {
        let sb = Sandbox::new();
        sb.configure();
        let mut config = Config::default();
        config.proxy.bind = "nonsense".to_owned();
        let err = assemble(&config, &sb.env.data.db_path(), "s", false, Timestamp(0))
            .err()
            .unwrap();
        assert!(err.to_string().contains("invalid [proxy]"), "{err}");
        let err = assemble(
            &Config::default(),
            &sb.path("no/such/dir/t.db"),
            "s",
            false,
            Timestamp(0),
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("could not open"), "{err}");
    }

    #[test]
    fn the_flush_loop_ends_when_shutdown_is_triggered() {
        let (_dir, _store) = store();
        let (sink, writer) = spawn_writer(Box::new(|_| Ok(())));
        let tracker = PromptTracker::new(
            tokred_core::pricing::PriceTable::default(),
            TrackerSettings::from_config(&Config::default().suggest),
            Arc::new(sink),
        );
        let shutdown = Shutdown::new();
        let stopper = shutdown.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            stopper.trigger();
        });
        let ticks = std::cell::Cell::new(0_u32);
        flush_loop(&tracker, &shutdown, Duration::from_millis(50), &|| {
            ticks.set(ticks.get() + 1);
        });
        assert!(ticks.get() >= 1, "the tick runs with every flush");
        drop(tracker);
        let _ = writer.join();
        assert!(shutdown.is_triggered());
    }
}

#[cfg(test)]
mod retention_tests {
    use super::super::testing::Sandbox;
    use super::*;

    fn request(id: &str, ts: i64) -> tokred_core::proxy::ProxyRequestRecord {
        tokred_core::proxy::ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            ..tokred_core::proxy::ProxyRequestRecord::default()
        }
    }

    fn ids(store: &Store) -> Vec<String> {
        store
            .proxy_requests_between(Timestamp(0), Timestamp(i64::MAX))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    #[test]
    fn old_data_is_deleted_unless_retention_is_zero_or_the_database_is_unusable() {
        let sb = Sandbox::new();
        sb.configure();
        let now = SystemClock.now().0;
        let store = sb.store();
        store
            .insert_proxy_request(&request("old", now - 100 * 86_400))
            .unwrap();
        store.insert_proxy_request(&request("new", now)).unwrap();

        prune_old_data(&sb.env.data.db_path(), 0);
        assert_eq!(ids(&store), ["old", "new"], "0 keeps everything");
        prune_old_data(&sb.env.data.db_path(), 90);
        assert_eq!(ids(&store), ["new"]);
        prune_old_data(&sb.path("no/such/dir/t.db"), 90);
    }

    #[test]
    fn the_prune_loop_ticks_at_once_and_then_on_a_schedule_until_shutdown() {
        let shutdown = Shutdown::new();
        let stopper = shutdown.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            stopper.trigger();
        });
        let ticks = std::cell::Cell::new(0_u32);
        prune_loop(&shutdown, Duration::from_millis(80), &|| {
            ticks.set(ticks.get() + 1);
        });
        assert!(ticks.get() >= 2, "{} ticks", ticks.get());
    }

    #[test]
    fn the_pruner_thread_prunes_at_start_and_ends_with_the_shutdown() {
        let sb = Sandbox::new();
        sb.configure();
        let now = SystemClock.now().0;
        sb.store()
            .insert_proxy_request(&request("old", now - 100 * 86_400))
            .unwrap();
        let shutdown = Shutdown::new();
        let pruner = spawn_pruner(
            shutdown.clone(),
            sb.env.data.db_path(),
            90,
            Duration::from_secs(3600),
        );
        std::thread::sleep(Duration::from_millis(400));
        shutdown.trigger();
        pruner.join().unwrap();
        assert!(ids(&sb.store()).is_empty());
    }
}

#[cfg(test)]
mod breaker_tests {
    use tokred_core::config::TrafficMode;
    use tokred_core::proxy::{HookNotes, Origin, ProxyHook, RequestCtx, RequestMeta, RequestShape};

    use super::super::advice::COMPRESSION_TRIPPED_AT;
    use super::super::testing::Sandbox;
    use super::*;

    fn tripped_hook() -> Arc<CompressionHook> {
        let (sink, _writer) = spawn_writer(Box::new(|_| Ok(())));
        let mut config = Config::default();
        config.compression.mode = TrafficMode::Enforce;
        let hook = Arc::new(CompressionHook::new(
            &config.compression,
            &config.shadow,
            true,
            Arc::new(sink),
        ));
        let meta = RequestMeta {
            session_id: Some("s".into()),
            ..RequestMeta::default()
        };
        let shape = RequestShape::default();
        let origin = Origin::Main;
        let ctx = RequestCtx {
            meta: &meta,
            shape: &shape,
            origin: &origin,
            now: Timestamp(1),
            body: serde_json::json!({}),
            changed: true,
            notes: HookNotes {
                blocks_compressed: 1,
                ..HookNotes::default()
            },
        };
        hook.fallback(&ctx, 400);
        hook
    }

    #[test]
    fn a_tripped_breaker_is_saved_once_and_forces_observe_on_restart() {
        let sb = Sandbox::new();
        sb.configure();
        let db = sb.env.data.db_path();
        let healthy = {
            let (sink, _w) = spawn_writer(Box::new(|_| Ok(())));
            let config = Config::default();
            CompressionHook::new(&config.compression, &config.shadow, true, Arc::new(sink))
        };
        persist_breaker(&healthy, &db);
        assert_eq!(
            sb.store().meta_get(COMPRESSION_TRIPPED_AT).unwrap(),
            None,
            "healthy: nothing saved"
        );

        let hook = tripped_hook();
        assert!(hook.is_tripped());
        persist_breaker(&hook, &db);
        let first = sb
            .store()
            .meta_get(COMPRESSION_TRIPPED_AT)
            .unwrap()
            .unwrap();
        assert!(!first.is_empty());
        persist_breaker(&hook, &db);
        assert_eq!(
            sb.store()
                .meta_get(COMPRESSION_TRIPPED_AT)
                .unwrap()
                .unwrap(),
            first,
            "kept, not rewritten"
        );
        persist_breaker(&hook, &sb.path("no/such/dir/t.db"));

        // The flusher saves a tripped breaker on its own, then ends with the shutdown.
        sb.store().meta_set(COMPRESSION_TRIPPED_AT, "").unwrap();
        let (sink, _w) = spawn_writer(Box::new(|_| Ok(())));
        let tracker = Arc::new(PromptTracker::new(
            tokred_core::pricing::PriceTable::default(),
            TrackerSettings::from_config(&Config::default().suggest),
            Arc::new(sink),
        ));
        let shutdown = Shutdown::new();
        let flusher = spawn_flusher(
            tracker,
            hook.clone(),
            shutdown.clone(),
            db.clone(),
            Duration::from_millis(50),
        );
        std::thread::sleep(Duration::from_millis(300));
        shutdown.trigger();
        flusher.join().unwrap();
        let saved = sb.store().meta_get(COMPRESSION_TRIPPED_AT).unwrap();
        assert!(saved.is_some_and(|at| !at.is_empty()));

        // Stored replacements load through the shared connection; a poisoned lock yields none.
        let shared = Mutex::new(Store::open(&db).unwrap());
        assert!(load_blocks(&shared, "no-such-conversation").is_empty());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = shared.lock().unwrap();
            panic!("poison the lock");
        }));
        assert!(load_blocks(&shared, "no-such-conversation").is_empty());

        let mut config = Config::default();
        config.compression.mode = TrafficMode::Enforce;
        apply_persisted_breaker(&mut config, &db);
        assert_eq!(config.compression.mode, TrafficMode::Observe);
        // Observe stays observe; a cleared mark leaves enforce alone.
        apply_persisted_breaker(&mut config, &db);
        assert_eq!(config.compression.mode, TrafficMode::Observe);
        sb.store().meta_set(COMPRESSION_TRIPPED_AT, "").unwrap();
        config.compression.mode = TrafficMode::Enforce;
        apply_persisted_breaker(&mut config, &db);
        assert_eq!(config.compression.mode, TrafficMode::Enforce);
    }
}
