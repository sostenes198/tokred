//! `guard-read` and `guard-search`: allow, deny, modes and error paths.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

mod support;

use serde_json::json;
use support::{CWD, Env, context_of, denial, note, tool};
use tokred_core::cache::CacheRepo;
use tokred_core::config::Mode;
use tokred_core::session::{FileReadRepo, SessionRepo};
use tokred_core::{HookEvent, Timestamp};
use tokred_hooks::{HookError, MAX_HASH_BYTES, handle};

fn read(session: &str, path: &str) -> serde_json::Value {
    tool(session, "Read", json!({ "file_path": path }))
}

fn read_range(session: &str, path: &str, offset: u64, limit: u64) -> serde_json::Value {
    tool(
        session,
        "Read",
        json!({ "file_path": path, "offset": offset, "limit": limit }),
    )
}

#[test]
fn lockfiles_and_generated_files_are_denied_with_a_short_reason() {
    let env = Env::new();
    for path in [
        "/work/proj/Cargo.lock",
        "/work/proj/web/package-lock.json",
        "/work/proj/dist/app.js",
        "/work/proj/node_modules/x/index.js",
        "/work/proj/static/app.min.js",
        "/work/proj/static/app.js.map",
        "/work/proj/target/debug/tokred",
        "/work/proj/.git/config",
    ] {
        env.files.sized(path, 1234);
        let response = env.run(HookEvent::GuardRead, &read("s", path));
        let reason = denial(&response).unwrap_or_else(|| panic!("{path} was not denied"));
        assert!(reason.starts_with("tokred: "), "{reason}");
        assert!(reason.len() < 200, "{reason}");
        assert_eq!(
            note(&response),
            ("deny".into(), "generated_file".into(), 1234),
            "{path}"
        );
    }
}

#[test]
fn a_project_that_lives_under_a_build_folder_is_not_denied() {
    let env = Env::new();
    env.files.sized("/home/u/build/proj/src/a.rs", 10);
    let payload = json!({
        "session_id": "s", "cwd": "/home/u/build/proj", "tool_name": "Read",
        "tool_input": {"file_path": "/home/u/build/proj/src/a.rs"},
    });
    let response = env.run(HookEvent::GuardRead, &payload);
    assert_eq!(denial(&response), None);
    assert_eq!(note(&response).0, "allow");
}

#[test]
fn windows_separators_are_normalized_before_matching() {
    let env = Env::new();
    let payload = json!({
        "session_id": "s", "tool_name": "Read",
        "tool_input": {"file_path": "C:\\proj\\node_modules\\x\\a.js"},
    });
    assert!(denial(&env.run(HookEvent::GuardRead, &payload)).is_some());
}

#[test]
fn deny_globs_come_from_the_config_and_invalid_ones_are_ignored() {
    let mut env = Env::new();
    env.config.guard_read.deny_globs = vec!["[".into(), "**/*.secret".into()];
    env.files.sized("/work/proj/a.secret", 5);
    env.files.sized("/work/proj/Cargo.lock", 5);
    assert!(denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.secret"))).is_some());
    // The default list was replaced, so Cargo.lock is fine now.
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/Cargo.lock"))),
        None
    );
}

#[test]
fn warn_mode_advises_without_denying_and_never_allows_explicitly() {
    let mut env = Env::new();
    env.config.modes.guard_read = Mode::Warn;
    env.files.sized("/work/proj/Cargo.lock", 99);
    let response = env.run(HookEvent::GuardRead, &read("s", "/work/proj/Cargo.lock"));
    assert_eq!(denial(&response), None);
    assert!(context_of(&response).unwrap().starts_with("tokred: "));
    let stdout = response.stdout.as_deref().unwrap();
    assert!(!stdout.contains("permissionDecision"), "{stdout}");
    // Nothing was avoided: the read goes ahead.
    assert_eq!(note(&response), ("warn".into(), "generated_file".into(), 0));
}

#[test]
fn off_mode_and_foreign_input_do_nothing() {
    let mut env = Env::new();
    env.files.sized("/work/proj/Cargo.lock", 1);
    env.config.modes.guard_read = Mode::Off;
    let off = env.run(HookEvent::GuardRead, &read("s", "/work/proj/Cargo.lock"));
    assert_eq!((off.stdout, off.note), (None, None));

    env.config.modes.guard_read = Mode::Enforce;
    let other_tool = tool("s", "Write", json!({"file_path": "/work/proj/Cargo.lock"}));
    assert_eq!(env.run(HookEvent::GuardRead, &other_tool).note, None);
    let no_path = tool("s", "Read", json!({}));
    assert_eq!(env.run(HookEvent::GuardRead, &no_path).note, None);
}

#[test]
fn missing_files_and_directories_are_left_to_the_tool() {
    let env = Env::new();
    env.files.dir("/work/proj/src", 1);
    for path in ["/work/proj/nope.rs", "/work/proj/src"] {
        let response = env.run(HookEvent::GuardRead, &read("s", path));
        assert_eq!(denial(&response), None);
        assert_eq!(note(&response).0, "allow");
    }
    assert_eq!(env.files.hash_calls.get(), 0);
}

#[test]
fn a_large_file_needs_a_range() {
    let env = Env::new();
    let max = env.config.guard_read.max_read_bytes as usize;
    env.files.sized("/work/proj/big.log", max + 1);
    env.files.sized("/work/proj/edge.log", max);

    let response = env.run(HookEvent::GuardRead, &read("s", "/work/proj/big.log"));
    let reason = denial(&response).unwrap();
    assert!(
        reason.contains("offset") && reason.contains("limit"),
        "{reason}"
    );
    assert_eq!(
        note(&response),
        ("deny".into(), "large_file".into(), max as u64 + 1)
    );

    // Exactly at the limit, or with any range, it is fine.
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/edge.log"))),
        None
    );
    let ranged = read_range("s", "/work/proj/big.log", 0, 100);
    assert_eq!(denial(&env.run(HookEvent::GuardRead, &ranged)), None);
    let only_limit = tool(
        "s",
        "Read",
        json!({"file_path": "/work/proj/big.log", "limit": 10}),
    );
    assert_eq!(denial(&env.run(HookEvent::GuardRead, &only_limit)), None);
    let only_offset = tool(
        "s",
        "Read",
        json!({"file_path": "/work/proj/big.log", "offset": 10}),
    );
    assert_eq!(denial(&env.run(HookEvent::GuardRead, &only_offset)), None);
}

#[test]
fn large_images_and_pdfs_are_not_blocked() {
    let env = Env::new();
    let size = env.config.guard_read.max_read_bytes as usize + 10;
    for path in [
        "/work/proj/shot.PNG",
        "/work/proj/doc.pdf",
        "/work/proj/a.jpeg",
    ] {
        env.files.sized(path, size);
        assert_eq!(
            denial(&env.run(HookEvent::GuardRead, &read("s", path))),
            None,
            "{path}"
        );
    }
}

#[test]
fn a_reread_of_unchanged_content_is_denied_and_the_bytes_are_counted() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"fn main() {}\n");
    let first = env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"));
    assert_eq!(denial(&first), None);
    assert_eq!(note(&first), ("allow".into(), String::new(), 0));

    env.advance(5);
    let second = env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"));
    let reason = denial(&second).unwrap();
    assert!(reason.contains("already in context"), "{reason}");
    assert_eq!(note(&second), ("deny".into(), "reread".into(), 13));

    // The denial does not refresh the recorded read.
    let hash = blake3::hash(b"fn main() {}\n").to_hex().to_string();
    let stored = env
        .store
        .find_read("s", "/work/proj/a.rs", &hash, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(stored.ts, Timestamp(1_000_000));
}

#[test]
fn changed_content_another_range_or_another_context_is_allowed() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"one");
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"))),
        None
    );

    // A ranged read is a different request.
    let ranged = read_range("s", "/work/proj/a.rs", 0, 10);
    assert_eq!(denial(&env.run(HookEvent::GuardRead, &ranged)), None);
    assert!(denial(&env.run(HookEvent::GuardRead, &ranged)).is_some());

    // Another session, and a subagent of the same session, have their own context.
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("other", "/work/proj/a.rs"))),
        None
    );
    let mut in_agent = read("s", "/work/proj/a.rs");
    in_agent["agent_id"] = json!("agent-1");
    assert_eq!(denial(&env.run(HookEvent::GuardRead, &in_agent)), None);
    assert!(denial(&env.run(HookEvent::GuardRead, &in_agent)).is_some());

    // New content (the file was edited) is a new read.
    env.files.file("/work/proj/a.rs", b"two");
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"))),
        None
    );
}

#[test]
fn a_clear_or_compact_cut_allows_the_read_again() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"one");
    env.advance(0);
    env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"));
    env.advance(10);
    assert!(denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"))).is_some());

    env.advance(10);
    let compact = json!({"session_id": "s", "cwd": CWD, "source": "compact"});
    env.run(HookEvent::SessionStart, &compact);

    // After the cut the same read is allowed once, then it is in context again.
    env.advance(1);
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"))),
        None
    );
    env.advance(1);
    assert!(denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"))).is_some());
}

#[test]
fn a_read_in_the_same_second_as_the_cut_is_allowed() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"one");
    env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"));
    env.store.mark_reset("s", Timestamp(1_000_000)).unwrap();
    // read.ts == cut: not provably in context, so it is allowed (fail-open side).
    assert_eq!(
        denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"))),
        None
    );
}

#[test]
fn warn_mode_flags_a_reread_but_still_records_it() {
    let mut env = Env::new();
    env.config.modes.guard_read = Mode::Warn;
    env.files.file("/work/proj/a.rs", b"one");
    env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"));
    env.advance(7);
    let second = env.run(HookEvent::GuardRead, &read("s", "/work/proj/a.rs"));
    assert!(context_of(&second).unwrap().contains("unchanged"));
    assert_eq!(note(&second), ("warn".into(), "reread".into(), 0));
    let hash = blake3::hash(b"one").to_hex().to_string();
    let stored = env
        .store
        .find_read("s", "/work/proj/a.rs", &hash, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(stored.ts, Timestamp(1_000_007));
}

#[test]
fn without_a_session_or_for_huge_files_there_is_no_reread_check() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"one");
    for _ in 0..2 {
        assert_eq!(
            denial(&env.run(HookEvent::GuardRead, &read("", "/work/proj/a.rs"))),
            None
        );
    }
    assert_eq!(env.files.hash_calls.get(), 0);

    let mut env = Env::new();
    env.config.guard_read.max_read_bytes = u64::MAX;
    env.files
        .sized("/work/proj/huge.bin", MAX_HASH_BYTES as usize + 1);
    for _ in 0..2 {
        assert_eq!(
            denial(&env.run(HookEvent::GuardRead, &read("s", "/work/proj/huge.bin"))),
            None
        );
    }
    assert_eq!(env.files.hash_calls.get(), 0);
}

#[test]
fn a_ranged_read_is_estimated_by_lines_and_capped_by_the_file_size() {
    let env = Env::new();
    env.files.sized("/work/proj/a.txt", 10_000);
    env.files.sized("/work/proj/b.txt", 100);
    for path in ["/work/proj/a.txt", "/work/proj/b.txt"] {
        let ranged = read_range("s", path, 0, 10);
        env.run(HookEvent::GuardRead, &ranged);
    }
    let a = env.run(
        HookEvent::GuardRead,
        &read_range("s", "/work/proj/a.txt", 0, 10),
    );
    let b = env.run(
        HookEvent::GuardRead,
        &read_range("s", "/work/proj/b.txt", 0, 10),
    );
    assert_eq!(note(&a).2, 800);
    assert_eq!(note(&b).2, 100);
}

#[test]
fn guard_read_errors_are_reported_to_the_runtime() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"one");
    let payload = read("s", "/work/proj/a.rs").to_string();

    env.files.fail_stat.set(true);
    assert!(matches!(
        handle(HookEvent::GuardRead, payload.as_bytes(), &env.deps()),
        Err(HookError::Io(_))
    ));
    env.files.fail_stat.set(false);

    env.files.fail_hash.set(true);
    assert!(handle(HookEvent::GuardRead, payload.as_bytes(), &env.deps()).is_err());
    env.files.fail_hash.set(false);

    assert!(handle(HookEvent::GuardRead, b"not json", &env.deps()).is_err());
    assert!(handle(HookEvent::GuardRead, b"", &env.deps()).is_err());
}

// ---------------------------------------------------------------- guard-search

fn grep(input: serde_json::Value) -> serde_json::Value {
    tool("s", "Grep", input)
}

fn big_repo() -> Env {
    let env = Env::new();
    env.files.dir("/work/proj/.git", 1);
    env.files.dir("/work/proj", 500);
    env.files.file_count.set(9000);
    env
}

#[test]
fn a_broad_search_in_a_large_repo_warns_by_default_and_denies_in_enforce() {
    let mut env = big_repo();
    let broad = grep(json!({"pattern": "TODO"}));

    let warned = env.run(HookEvent::GuardSearch, &broad);
    assert_eq!(denial(&warned), None);
    let text = context_of(&warned).unwrap();
    assert!(text.contains("5000") && text.contains("path"), "{text}");
    assert_eq!(note(&warned), ("warn".into(), "broad_search".into(), 0));

    env.config.modes.guard_search = Mode::Enforce;
    let denied = env.run(HookEvent::GuardSearch, &broad);
    assert!(denial(&denied).unwrap().contains("glob"));
    assert_eq!(note(&denied).0, "deny");
}

#[test]
fn the_file_count_is_cached_per_root_and_promoted_on_reads() {
    let env = big_repo();
    let broad = grep(json!({"pattern": "TODO"}));
    for _ in 0..4 {
        env.run(HookEvent::GuardSearch, &broad);
    }
    assert_eq!(env.files.count_calls.get(), 1);
    let entries = env.store.list_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "repo_files");
    assert_eq!(entries[0].value, b"5001");
    assert_eq!(entries[0].hits, 3);
    assert_eq!(entries[0].tier, tokred_core::cache::Tier::L2);

    // A changed root (new modification time) is counted again; so is an expired entry.
    env.files.dir("/work/proj", 501);
    env.run(HookEvent::GuardSearch, &broad);
    assert_eq!(env.files.count_calls.get(), 2);
    env.advance(5 * 3600);
    env.run(HookEvent::GuardSearch, &broad);
    assert_eq!(env.files.count_calls.get(), 3);
}

#[test]
fn a_corrupt_cached_count_is_recounted() {
    let env = big_repo();
    let broad = grep(json!({"pattern": "TODO"}));
    env.run(HookEvent::GuardSearch, &broad);
    let mut entry = env.store.list_entries().unwrap().remove(0);
    entry.value = b"not a number".to_vec();
    env.store.put_entry(&entry).unwrap();
    let response = env.run(HookEvent::GuardSearch, &broad);
    assert_eq!(note(&response).1, "broad_search");
    assert_eq!(env.files.count_calls.get(), 2);
}

#[test]
fn small_repos_and_narrow_searches_are_allowed_without_counting() {
    let env = big_repo();
    env.files.file_count.set(10);
    let small = env.run(HookEvent::GuardSearch, &grep(json!({"pattern": "TODO"})));
    assert_eq!(note(&small).0, "allow");
    assert_eq!(env.files.count_calls.get(), 1);

    let env = big_repo();
    let narrow = [
        grep(json!({"pattern": "x", "path": "/work/proj/src"})),
        grep(json!({"pattern": "x", "path": "src"})),
        grep(json!({"pattern": "x", "glob": "*.rs"})),
        grep(json!({"pattern": "x", "type": "rust"})),
        tool("s", "Glob", json!({"pattern": "**/*.ts"})),
        tool(
            "s",
            "Glob",
            json!({"pattern": "*", "path": "/work/proj/web"}),
        ),
    ];
    for payload in &narrow {
        assert_eq!(
            note(&env.run(HookEvent::GuardSearch, payload)).0,
            "allow",
            "{payload}"
        );
    }
    assert_eq!(env.files.count_calls.get(), 0);
}

#[test]
fn searching_the_repo_root_explicitly_or_with_match_all_patterns_is_broad() {
    let env = big_repo();
    let broad = [
        grep(json!({"pattern": "x", "path": "/work/proj"})),
        grep(json!({"pattern": "x", "path": ""})),
        grep(json!({"pattern": "x", "glob": "  "})),
        tool("s", "Glob", json!({"pattern": "**/*"})),
        tool("s", "Glob", json!({"pattern": " * "})),
        tool("s", "Glob", json!({})),
    ];
    for payload in &broad {
        assert_eq!(
            note(&env.run(HookEvent::GuardSearch, payload)).1,
            "broad_search",
            "{payload}"
        );
    }
}

#[test]
fn the_root_is_the_nearest_ancestor_with_a_git_folder() {
    let env = big_repo();
    let from_subdir = json!({
        "session_id": "s", "cwd": "/work/proj/src/deep", "tool_name": "Grep",
        "tool_input": {"pattern": "x"},
    });
    assert_eq!(
        note(&env.run(HookEvent::GuardSearch, &from_subdir)).1,
        "broad_search"
    );

    // No repository at all: the search directory itself is the root.
    let env = Env::new();
    env.files.file_count.set(9000);
    let response = env.run(HookEvent::GuardSearch, &grep(json!({"pattern": "x"})));
    assert_eq!(note(&response).1, "broad_search");
}

#[test]
fn the_threshold_comes_from_the_config() {
    let mut env = big_repo();
    env.config.guard_search.large_repo_files = 10_000;
    let response = env.run(HookEvent::GuardSearch, &grep(json!({"pattern": "x"})));
    assert_eq!(note(&response).0, "allow");
}

#[test]
fn guard_search_ignores_other_tools_off_mode_and_missing_cwd() {
    let mut env = big_repo();
    assert_eq!(
        env.run(HookEvent::GuardSearch, &tool("s", "Read", json!({})))
            .note,
        None
    );
    let no_cwd = json!({"session_id": "s", "tool_name": "Grep", "tool_input": {"pattern": "x"}});
    assert_eq!(note(&env.run(HookEvent::GuardSearch, &no_cwd)).0, "allow");
    env.config.modes.guard_search = Mode::Off;
    let off = env.run(HookEvent::GuardSearch, &grep(json!({"pattern": "x"})));
    assert_eq!((off.stdout, off.note), (None, None));
}

#[test]
fn guard_search_errors_are_reported_to_the_runtime() {
    let env = big_repo();
    let payload = grep(json!({"pattern": "x"})).to_string();
    env.files.fail_count.set(true);
    assert!(handle(HookEvent::GuardSearch, payload.as_bytes(), &env.deps()).is_err());
    env.files.fail_count.set(false);
    env.files.fail_stat.set(true);
    assert!(handle(HookEvent::GuardSearch, payload.as_bytes(), &env.deps()).is_err());
}

#[test]
fn reads_with_extra_arguments_such_as_pdf_pages_are_never_compared() {
    let env = Env::new();
    env.files.file("/work/proj/doc.pdf", b"%PDF-1.7 pages");
    for pages in ["1-5", "6-10", "1-5"] {
        let payload = tool(
            "s",
            "Read",
            json!({"file_path": "/work/proj/doc.pdf", "pages": pages}),
        );
        let response = env.run(HookEvent::GuardRead, &payload);
        assert_eq!(denial(&response), None, "pages {pages}");
        assert_eq!(note(&response).0, "allow");
    }
    assert_eq!(env.files.hash_calls.get(), 0);
}
