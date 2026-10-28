//! `guard-read` and `guard-search` (`PreToolUse` on `Read` and on `Grep|Glob`).
//!
//! In `enforce` mode a denial returns `permissionDecision: "deny"` with a short reason that
//! tells the model what to do instead. In `warn` mode the same text is added as
//! `additionalContext` and the call proceeds; `permissionDecision: "allow"` is never emitted
//! because it would skip the user's permission prompt.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use tokred_core::cache::{TierPolicy, cache_get, cache_put, content_key};
use tokred_core::config::{GuardRead, Mode};
use tokred_core::hook_event::{decision, reason};
use tokred_core::session::FileRead;

use crate::error::HookError;
use crate::io::{HookInput, context, deny};
use crate::ports::{FileMeta, HookDeps, HookResponse};

/// Files larger than this are not hashed for the re-read check (it would cost too much time).
pub const MAX_HASH_BYTES: u64 = 16 * 1024 * 1024;

/// Average line length used to estimate the size of a ranged read.
const AVG_LINE_BYTES: u64 = 80;

/// Kind of the file-count rows in the cache.
const REPO_FILES_KIND: &str = "repo_files";

/// Extensions that the `Read` tool shows as images or documents: the size limit does not apply.
const MEDIA_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "pdf"];

/// Answers a violation according to the mode (never called with [`Mode::Off`]).
fn verdict(
    mode: Mode,
    session_id: &str,
    reason: &'static str,
    message: &str,
    bytes: u64,
) -> HookResponse {
    if mode == Mode::Enforce {
        HookResponse::noted(
            session_id,
            decision::DENY,
            reason,
            bytes,
            Some(deny(message)),
        )
    } else {
        // Nothing is avoided in `warn` mode: the call goes ahead.
        HookResponse::noted(
            session_id,
            decision::WARN,
            reason,
            0,
            Some(context("PreToolUse", message)),
        )
    }
}

fn allowed(session_id: &str) -> HookResponse {
    HookResponse::noted(session_id, decision::ALLOW, "", 0, None)
}

fn compile_globs(config: &GuardRead) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in &config.deny_globs {
        match GlobBuilder::new(pattern).literal_separator(true).build() {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(err) => tracing::warn!(%pattern, error = %err, "ignoring invalid deny glob"),
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Path used for glob matching: relative to `cwd` when inside it, with `/` separators, so
/// `**/build/**` does not match a project that merely lives under a folder called `build`.
fn match_target(path: &str, cwd: Option<&str>) -> String {
    let relative = cwd
        .and_then(|cwd| Path::new(path).strip_prefix(cwd).ok())
        .map_or_else(
            || path.to_owned(),
            |rest| rest.to_string_lossy().into_owned(),
        );
    relative.replace('\\', "/")
}

fn is_media(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| MEDIA_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn estimated_read_bytes(size: u64, limit: Option<u64>) -> u64 {
    limit.map_or(size, |lines| size.min(lines.saturating_mul(AVG_LINE_BYTES)))
}

/// `guard-read` (spec 7.3).
pub fn guard_read(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let mode = deps.config.modes.guard_read;
    if mode == Mode::Off || input.tool_name.as_deref() != Some("Read") {
        return Ok(HookResponse::silent());
    }
    let Some(path) = input.tool_str("file_path") else {
        return Ok(HookResponse::silent());
    };
    let session = input.session_id.as_str();
    let config = &deps.config.guard_read;
    let stat = deps.files.stat(Path::new(path))?;
    let size = stat.map_or(0, |s| s.size);
    let (offset, limit) = (input.tool_u64("offset"), input.tool_u64("limit"));

    // 1. Lockfiles, generated and vendored files.
    let target = match_target(path, input.cwd.as_deref());
    if compile_globs(config).is_match(&target) {
        let message = format!(
            "tokred: `{target}` is a lockfile or generated/vendored content. Read the source \
             files instead, or Grep for the exact line you need."
        );
        return Ok(verdict(
            mode,
            session,
            reason::GENERATED_FILE,
            &message,
            size,
        ));
    }

    // Missing files and directories are left for the tool to report.
    let Some(stat) = stat.filter(|s| !s.is_dir) else {
        return Ok(allowed(session));
    };

    // 2. Large file without a range.
    if stat.size > config.max_read_bytes && offset.is_none() && limit.is_none() && !is_media(path) {
        let message = format!(
            "tokred: `{target}` is {} KB (limit {} KB). Read it with `offset` and `limit`, or \
             Grep for what you need.",
            stat.size / 1024,
            config.max_read_bytes / 1024
        );
        return Ok(verdict(
            mode,
            session,
            reason::LARGE_FILE,
            &message,
            stat.size,
        ));
    }

    // 3. Re-read of unchanged content (needs a session, and a file small enough to hash).
    // Any argument beyond `file_path`, `offset` and `limit` (such as `pages` for PDFs) changes
    // what the read returns without changing the key, so those reads are never compared.
    let extra_arguments = input
        .tool_input
        .as_ref()
        .and_then(|v| v.as_object())
        .is_some_and(|args| {
            args.keys()
                .any(|key| !matches!(key.as_str(), "file_path" | "offset" | "limit"))
        });
    if session.is_empty() || stat.size > MAX_HASH_BYTES || extra_arguments {
        return Ok(allowed(session));
    }
    let hash = deps.files.hash(Path::new(path))?;
    let scope = input.scope();
    let cut = deps
        .sessions
        .get_session(session)?
        .and_then(|s| s.last_reset_at);
    let previous = deps.reads.find_read(&scope, path, &hash, offset, limit)?;
    // A read at or before the cut is no longer in the model's context (compaction or /clear).
    let in_context = previous.filter(|read| cut.is_none_or(|cut| read.ts > cut));

    let now = deps.clock.now();
    let bytes = estimated_read_bytes(stat.size, limit);
    let response = match &in_context {
        Some(read) => {
            let message = format!(
                "tokred: `{target}` is unchanged since you read it earlier in this session; its \
                 content is already in context. Use it from there."
            );
            verdict(mode, session, reason::REREAD, &message, read.bytes)
        }
        None => allowed(session),
    };
    if in_context.is_none() || mode != Mode::Enforce {
        // The read goes ahead (first read, after a cut, or `warn` mode): remember it.
        deps.reads.record_read(&FileRead {
            session_id: scope,
            path: path.to_owned(),
            hash,
            offset,
            limit,
            bytes,
            ts: now,
        })?;
    }
    Ok(response)
}

/// Nearest ancestor of `start` that holds a `.git`, or `fallback`.
fn repo_root(files: &dyn FileMeta, start: &Path, fallback: &Path) -> Result<PathBuf, HookError> {
    for dir in start.ancestors() {
        if files.stat(&dir.join(".git"))?.is_some() {
            return Ok(dir.to_path_buf());
        }
    }
    Ok(fallback.to_path_buf())
}

/// Patterns of `Glob` that match every file.
fn is_match_all(pattern: &str) -> bool {
    matches!(pattern.trim(), "*" | "**" | "**/*" | "**/*.*" | "*.*")
}

/// `guard-search` (spec 7.4).
pub fn guard_search(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let mode = deps.config.modes.guard_search;
    let tool = input.tool_name.as_deref();
    if mode == Mode::Off || !matches!(tool, Some("Grep" | "Glob")) {
        return Ok(HookResponse::silent());
    }
    let session = input.session_id.as_str();
    let Some(cwd) = input.cwd.as_deref().map(Path::new) else {
        return Ok(allowed(session));
    };

    // Where the search starts: `path` (absolute, or relative to the cwd), else the cwd.
    let given = input
        .tool_str("path")
        .filter(|p| !p.is_empty())
        .map(|p| cwd.join(p));
    let start = given.clone().unwrap_or_else(|| cwd.to_path_buf());
    let root = repo_root(deps.files, &start, &start)?;

    let at_root = given.as_ref().is_none_or(|p| *p == root);
    let restricted = if tool == Some("Grep") {
        ["glob", "type"]
            .iter()
            .any(|key| input.tool_str(key).is_some_and(|v| !v.trim().is_empty()))
    } else {
        !input.tool_str("pattern").is_none_or(is_match_all)
    };
    if !at_root || restricted {
        return Ok(allowed(session));
    }

    let threshold = deps.config.guard_search.large_repo_files;
    let files = repo_file_count(deps, &root, threshold)?;
    if files <= threshold {
        return Ok(allowed(session));
    }
    let message = format!(
        "tokred: this repository has more than {threshold} files. Narrow the search: pass \
         `path` (a subdirectory) or a `glob`/`type` filter."
    );
    Ok(verdict(mode, session, reason::BROAD_SEARCH, &message, 0))
}

/// Number of files under `root`, capped at `threshold + 1`, cached per root (and per threshold
/// and root modification time, so a changed repository eventually re-counts).
fn repo_file_count(deps: &HookDeps<'_>, root: &Path, threshold: u64) -> Result<u64, HookError> {
    let now = deps.clock.now();
    let policy = TierPolicy::from_config(&deps.config.cache);
    let mtime = deps.files.stat(root)?.map_or(0, |s| s.mtime);
    let key = content_key(
        REPO_FILES_KIND,
        &root.to_string_lossy(),
        &format!("t{threshold}-m{mtime}"),
    );
    if let Some(entry) = cache_get(deps.cache, &key, now, &policy)?
        && let Some(count) = std::str::from_utf8(&entry.value)
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
    {
        return Ok(count);
    }
    let count = deps.files.count_files(root, threshold.saturating_add(1))?;
    let stored = cache_put(
        deps.cache,
        &key,
        REPO_FILES_KIND,
        count.to_string().into_bytes(),
        now,
        &policy,
    );
    stored?;
    Ok(count)
}
