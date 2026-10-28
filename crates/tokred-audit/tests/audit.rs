#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end behaviour of `audit` over in-memory ports.

mod common;

use std::path::PathBuf;

use common::{FakeUsage, MemFs, NOW, has_warning, item, request, run};
use tokred_audit::{ItemKind, McpScope, Scope, global_config_candidates};

const USER_MD: &str = "/h/.claude/CLAUDE.md";

fn quiet() -> FakeUsage {
    FakeUsage::ok(&[])
}

fn lines(n: usize) -> String {
    "line\n".repeat(n)
}

#[test]
fn empty_setup_yields_an_empty_report() {
    let report = run(&MemFs::new(), &quiet(), &request(true));
    assert!(report.items.is_empty());
    assert!(report.mcp_servers.is_empty());
    assert!(report.warnings.is_empty());
    assert_eq!(report.summary.total_estimated_tokens, 0);
    assert_eq!(report.summary.by_kind.len(), 4);
    assert!(report.token_estimate.contains("estimate"));
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("cannot be computed locally"))
    );
}

#[test]
fn weighs_user_and_project_memory_files() {
    let fs = MemFs::new()
        .file(USER_MD, &"x".repeat(400))
        .file("/h/proj/CLAUDE.md", &"y".repeat(40))
        .file("/h/proj/.claude/CLAUDE.md", "abcd")
        .file("/h/proj/CLAUDE.local.md", "ab")
        .file("/h/CLAUDE.md", "ancestor");
    let report = run(&fs, &quiet(), &request(true));
    assert_eq!(item(&report, USER_MD).estimated_tokens, 100);
    assert_eq!(item(&report, USER_MD).scope, Scope::User);
    assert_eq!(item(&report, "/h/proj/CLAUDE.md").estimated_tokens, 10);
    assert_eq!(item(&report, "/h/proj/CLAUDE.md").scope, Scope::Project);
    assert_eq!(
        item(&report, "/h/proj/.claude/CLAUDE.md").estimated_tokens,
        1
    );
    assert_eq!(item(&report, "/h/proj/CLAUDE.local.md").estimated_tokens, 1);
    assert_eq!(item(&report, "/h/CLAUDE.md").estimated_tokens, 2);
    let paths: Vec<_> = report.items.iter().map(|i| i.estimated_tokens).collect();
    assert_eq!(paths, [100, 10, 2, 1, 1]);
    assert_eq!(report.summary.by_kind[&ItemKind::ClaudeMd].count, 5);
    assert_eq!(report.summary.total_estimated_tokens, 114);
}

#[test]
fn without_a_project_only_user_level_items_are_read() {
    let fs = MemFs::new()
        .file(USER_MD, "user")
        .file("/h/proj/CLAUDE.md", "project")
        .file("/h/proj/.claude/skills/s/SKILL.md", "skill");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 1);
}

#[test]
fn large_and_empty_memory_files_get_actions() {
    let fs = MemFs::new()
        .file(USER_MD, &lines(201))
        .file("/h/proj/CLAUDE.md", "  \n")
        .file("/h/proj/CLAUDE.local.md", &lines(200));
    let report = run(&fs, &quiet(), &request(true));
    let big = item(&report, USER_MD).action.clone().unwrap();
    assert!(big.contains("200 lines") && big.contains("does not reduce context cost"));
    assert!(
        item(&report, "/h/proj/CLAUDE.md")
            .action
            .as_deref()
            .unwrap()
            .contains("empty")
    );
    assert_eq!(item(&report, "/h/proj/CLAUDE.local.md").action, None);
}

#[test]
fn files_over_four_mebibytes_are_flagged() {
    let fs = MemFs::new().file(USER_MD, &"a".repeat(4 * 1024 * 1024 + 1));
    let report = run(&fs, &quiet(), &request(false));
    assert!(
        item(&report, USER_MD)
            .action
            .as_deref()
            .unwrap()
            .contains("skips")
    );
}

#[test]
fn block_comments_do_not_count() {
    let fs = MemFs::new().file(USER_MD, "<!-- 4444444444444444 -->\nabcd\n");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(item(&report, USER_MD).estimated_tokens, 2);
}

#[test]
fn nested_imports_resolve_relative_to_the_importing_file() {
    let fs = MemFs::new()
        .file(USER_MD, "@docs/a.md and @~/shared/home.md and @/h/abs.md")
        .file("/h/.claude/docs/a.md", "see @sub/b.md")
        .file("/h/.claude/docs/sub/b.md", "leaf!!!")
        .file("/h/shared/home.md", "home")
        .file("/h/abs.md", "abs");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 5);
    let b = item(&report, "/h/.claude/docs/sub/b.md");
    assert_eq!(b.imported_by, Some(PathBuf::from("/h/.claude/docs/a.md")));
    assert_eq!(b.name, "sub/b.md");
    assert_eq!(b.kind, ItemKind::ClaudeMd);
    assert_eq!(
        item(&report, "/h/shared/home.md").imported_by,
        Some(PathBuf::from(USER_MD))
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn import_cycles_are_reported_and_loaded_once() {
    let fs = MemFs::new()
        .file(USER_MD, "@a.md")
        .file("/h/.claude/a.md", "@b.md")
        .file("/h/.claude/b.md", "@a.md @CLAUDE.md");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 3);
    assert!(has_warning(&report, "import cycle"));
}

#[test]
fn a_file_imported_twice_is_counted_once() {
    let fs = MemFs::new()
        .file(USER_MD, "@a.md @b.md")
        .file("/h/.claude/a.md", "@shared.md")
        .file("/h/.claude/b.md", "@shared.md")
        .file("/h/.claude/shared.md", "shared");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 4);
    assert!(report.warnings.is_empty());
}

#[test]
fn missing_import_targets_warn_only_when_they_look_like_paths() {
    let fs = MemFs::new().file(
        USER_MD,
        "@missing/file.md @param and @~/none.md and @none.md.",
    );
    let mut req = request(false);
    req.home_dir = None;
    let report = run(&fs, &quiet(), &req);
    assert_eq!(report.items.len(), 1);
    assert!(has_warning(&report, "`@missing/file.md`"));
    assert!(has_warning(&report, "`@~/none.md`"));
    assert!(has_warning(&report, "`@none.md.`"));
    assert!(!has_warning(&report, "@param"));
}

#[test]
fn trailing_punctuation_is_tolerated_in_imports() {
    let fs = MemFs::new()
        .file(USER_MD, "read @notes.md.")
        .file("/h/.claude/notes.md", "n");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 2);
    assert_eq!(item(&report, "/h/.claude/notes.md").name, "notes.md.");
}

#[test]
fn imports_stop_after_four_hops() {
    let fs = MemFs::new()
        .file(USER_MD, "@l1.md")
        .file("/h/.claude/l1.md", "@l2.md")
        .file("/h/.claude/l2.md", "@l3.md")
        .file("/h/.claude/l3.md", "@l4.md")
        .file("/h/.claude/l4.md", "@l5.md @word")
        .file("/h/.claude/l5.md", "too deep");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 5);
    assert!(report.items.iter().all(|i| !i.path.ends_with("l5.md")));
    assert!(has_warning(&report, "more than 4 hops"));
    assert!(!has_warning(&report, "@word"));
}

#[test]
fn imports_in_code_are_ignored() {
    let fs = MemFs::new()
        .file(USER_MD, "```\n@fenced.md\n```\n`@span.md`\n")
        .file("/h/.claude/fenced.md", "x")
        .file("/h/.claude/span.md", "x");
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.items.len(), 1);
    assert!(report.warnings.is_empty());
}

#[test]
fn unreadable_files_become_warnings() {
    let fs = MemFs::new()
        .file(USER_MD, "@bad.md")
        .unreadable("/h/.claude/bad.md")
        .unreadable("/h/proj/CLAUDE.md")
        .unlistable("/h/.claude/agents");
    let report = run(&fs, &quiet(), &request(true));
    assert_eq!(report.items.len(), 1);
    assert_eq!(
        report
            .warnings
            .iter()
            .filter(|w| w.contains("denied"))
            .count(),
        3,
        "{:?}",
        report.warnings
    );
    assert!(!has_warning(&report, "not found"));
}

#[test]
fn skills_are_weighed_with_frontmatter_and_listing_cost() {
    let skill = "---\nname: deploy-helper\ndescription: Deploys things\nwhen_to_use: When shipping\n---\nbody body body\n";
    let fs = MemFs::new()
        .file("/h/.claude/skills/deploy/SKILL.md", skill)
        .file("/h/.claude/skills/plain/SKILL.md", "no frontmatter")
        .file("/h/.claude/skills/.trash/old/SKILL.md", "hidden")
        .file("/h/.claude/skills/empty/notes.txt", "no skill file")
        .file("/h/.claude/skills/loose.md", "not a dir")
        .file(
            "/h/proj/.claude/skills/local/SKILL.md",
            "---\nname: local-skill\n---\nx",
        );
    let report = run(&fs, &quiet(), &request(true));
    let skills: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Skill)
        .collect();
    assert_eq!(skills.len(), 3);
    let deploy = item(&report, "/h/.claude/skills/deploy/SKILL.md");
    assert_eq!(deploy.name, "deploy-helper");
    assert_eq!(deploy.estimated_tokens, estimate(skill));
    assert_eq!(
        deploy.startup_tokens,
        Some(estimate("Deploys thingsWhen shipping"))
    );
    assert_eq!(
        item(&report, "/h/.claude/skills/plain/SKILL.md").name,
        "plain"
    );
    assert_eq!(
        item(&report, "/h/proj/.claude/skills/local/SKILL.md").scope,
        Scope::Project
    );
    assert_eq!(report.summary.by_kind[&ItemKind::Skill].count, 3);
}

fn estimate(text: &str) -> u64 {
    text.chars().count().div_ceil(4) as u64
}

#[test]
fn skill_thresholds_and_frontmatter_problems_are_flagged() {
    let long_desc = format!("---\nname: wordy\ndescription: {}\n---\n", "d".repeat(1537));
    let fs = MemFs::new()
        .file("/h/.claude/skills/long/SKILL.md", &lines(501))
        .file("/h/.claude/skills/wordy/SKILL.md", &long_desc)
        .file(
            "/h/.claude/skills/broken/SKILL.md",
            "---\nname: broken\nno closing fence\n",
        )
        .file("/h/.claude/skills/blank/SKILL.md", "\n\n")
        .file(
            "/h/.claude/skills/fine/SKILL.md",
            "---\nname: fine\n---\nshort\n",
        );
    let report = run(&fs, &quiet(), &request(false));
    let action = |dir: &str| {
        item(&report, &format!("/h/.claude/skills/{dir}/SKILL.md"))
            .action
            .clone()
    };
    assert!(action("long").unwrap().contains("500 lines"));
    assert!(action("wordy").unwrap().contains("truncated"));
    assert!(action("broken").unwrap().contains("frontmatter"));
    assert!(action("blank").unwrap().contains("empty"));
    assert_eq!(action("fine"), None);
    assert_eq!(
        item(&report, "/h/.claude/skills/wordy/SKILL.md").startup_tokens,
        Some(384)
    );
    assert!(has_warning(&report, "invalid frontmatter"));
}

#[test]
fn agents_are_found_recursively_and_named_from_frontmatter() {
    let big = format!("---\nname: huge\n---\n{}", "w".repeat(8004));
    let fs = MemFs::new()
        .file(
            "/h/.claude/agents/reviewer.md",
            "---\nname: code-reviewer\ndescription: reviews\n---\nprompt",
        )
        .file("/h/.claude/agents/team/planner.md", "no frontmatter")
        .file("/h/.claude/agents/team/huge.md", &big)
        .file("/h/.claude/agents/team/notes.txt", "ignored")
        .file("/h/.claude/agents/.hidden/x.md", "ignored")
        .file("/h/proj/.claude/agents/mine.md", "---\nname: \n---\nx");
    let report = run(&fs, &quiet(), &request(true));
    let agents: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Agent)
        .collect();
    assert_eq!(agents.len(), 4);
    assert_eq!(
        item(&report, "/h/.claude/agents/reviewer.md").name,
        "code-reviewer"
    );
    assert_eq!(
        item(&report, "/h/.claude/agents/team/planner.md").name,
        "planner"
    );
    assert_eq!(item(&report, "/h/proj/.claude/agents/mine.md").name, "mine");
    assert_eq!(
        item(&report, "/h/.claude/agents/reviewer.md").startup_tokens,
        None
    );
    assert!(
        item(&report, "/h/.claude/agents/team/huge.md")
            .action
            .as_deref()
            .unwrap()
            .contains("trimming")
    );
    assert_eq!(item(&report, "/h/.claude/agents/reviewer.md").action, None);
}

#[test]
fn rules_without_paths_count_and_path_scoped_rules_do_not() {
    let fs = MemFs::new()
        .file("/h/.claude/rules/style.md", "always on")
        .file(
            "/h/proj/.claude/rules/api/scoped.md",
            "---\npaths:\n  - \"src/**\"\n---\nonly sometimes",
        )
        .file(
            "/h/proj/.claude/rules/api/always.md",
            "---\ndescription: x\n---\n@../../extra.md",
        )
        .file("/h/proj/.claude/extra.md", "extra");
    let report = run(&fs, &quiet(), &request(true));
    let rules: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Rule)
        .collect();
    assert_eq!(rules.len(), 3, "{:?}", report.items);
    assert!(report.items.iter().all(|i| !i.path.ends_with("scoped.md")));
    assert_eq!(
        item(&report, "/h/proj/.claude/extra.md").kind,
        ItemKind::Rule
    );
}

#[test]
fn items_are_ordered_by_weight_with_a_stable_tie_break() {
    let fs = MemFs::new()
        .file("/h/.claude/agents/b.md", "1234")
        .file("/h/.claude/agents/a.md", "1234")
        .file("/h/.claude/skills/s/SKILL.md", "1234")
        .file("/h/.claude/rules/r.md", "1234")
        .file(USER_MD, "1234")
        .file("/h/proj/CLAUDE.md", "1234")
        .file("/h/.claude/skills/big/SKILL.md", "123456789");
    let report = run(&fs, &quiet(), &request(true));
    let order: Vec<String> = report
        .items
        .iter()
        .map(|i| {
            i.path
                .strip_prefix("/h")
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(
        order,
        [
            ".claude/skills/big/SKILL.md",
            ".claude/CLAUDE.md",
            "proj/CLAUDE.md",
            ".claude/rules/r.md",
            ".claude/skills/s/SKILL.md",
            ".claude/agents/a.md",
            ".claude/agents/b.md",
        ]
    );
}

#[test]
fn the_user_claude_md_is_not_counted_twice_when_the_project_is_the_home() {
    let fs = MemFs::new()
        .file(USER_MD, "same file")
        .file("/h/.claude/skills/s/SKILL.md", "skill")
        .file("/h/.claude/agents/a.md", "agent");
    let mut req = request(true);
    req.project_dir = Some(PathBuf::from("/h"));
    let report = run(&fs, &quiet(), &req);
    assert_eq!(report.items.len(), 3);
    assert!(report.items.iter().all(|i| i.scope == Scope::User));
}

// ---- MCP ----

const GLOBAL: &str = "/h/.claude.json";

#[test]
fn mcp_scopes_precedence_and_usage() {
    let global = r#"{
        "mcpServers": {"gh": {"type": "http", "url": "u"}, "shared": {"command": "x"}, "Claude Browser": {"command": "c"}},
        "projects": {
            "/h/proj": {"mcpServers": {"shared": {"command": "local"}, "secret": {"command": "s"}}},
            "/h/other": {"mcpServers": {"elsewhere": {"command": "e"}}}
        }
    }"#;
    let fs = MemFs::new().file(GLOBAL, global).file(
        "/h/proj/.mcp.json",
        r#"{"mcpServers": {"shared": {"command": "p"}, "team": {"type": "sse", "url": "u"}}}"#,
    );
    let usage = FakeUsage::ok(&[
        ("gh", 5, Some(NOW - 10)),
        ("Claude_Browser", 2, None),
        ("plugin_x_y", 7, Some(NOW)),
    ]);
    let report = run(&fs, &usage, &request(true));
    assert_eq!(usage.requested.get(), Some(NOW - 30 * 86_400));
    let by_name = |name: &str| report.mcp_servers.iter().find(|s| s.name == name).unwrap();
    assert_eq!(by_name("shared").scope, McpScope::Local);
    assert_eq!(
        by_name("shared").also_in,
        [McpScope::Project, McpScope::User]
    );
    assert_eq!(by_name("shared").source, PathBuf::from(GLOBAL));
    assert_eq!(by_name("team").scope, McpScope::Project);
    assert_eq!(by_name("team").transport, "sse");
    assert_eq!(by_name("gh").scope, McpScope::User);
    assert_eq!(by_name("gh").transport, "http");
    assert_eq!(by_name("gh").calls, 5);
    assert_eq!(by_name("gh").last_seen_unix, Some(NOW - 10));
    assert_eq!(by_name("Claude Browser").calls, 2);
    assert!(!report.mcp_servers.iter().any(|s| s.name == "elsewhere"));
    let unused: Vec<_> = report
        .mcp_servers
        .iter()
        .filter(|s| s.unused)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(unused, ["secret", "shared", "team"]);
    assert_eq!(report.summary.mcp_servers, 5);
    assert_eq!(report.summary.mcp_servers_unused, 3);
    assert_eq!(report.mcp_usage_without_local_config.len(), 1);
    assert_eq!(
        report.mcp_usage_without_local_config[0].server,
        "plugin_x_y"
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn project_scope_servers_need_a_project_and_a_global_config_may_be_absent() {
    let fs = MemFs::new()
        .file(
            GLOBAL,
            r#"{"mcpServers": {"a": {}}, "projects": {"/h/proj": {"mcpServers": {"b": {}}}}}"#,
        )
        .file("/h/proj/.mcp.json", r#"{"mcpServers": {"c": {}}}"#);
    let report = run(&fs, &quiet(), &request(false));
    let names: Vec<_> = report.mcp_servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["a"]);
    let report = run(&MemFs::new(), &quiet(), &request(true));
    assert!(report.mcp_servers.is_empty());
}

#[test]
fn config_dir_copy_of_claude_json_wins_over_the_home_copy() {
    let fs = MemFs::new()
        .file(
            "/h/.claude/.claude.json",
            r#"{"mcpServers": {"from-config-dir": {}}}"#,
        )
        .file(GLOBAL, r#"{"mcpServers": {"from-home": {}}}"#);
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.mcp_servers.len(), 1);
    assert_eq!(report.mcp_servers[0].name, "from-config-dir");
    assert_eq!(
        global_config_candidates("/h/.claude".as_ref(), Some("/h".as_ref())),
        [
            PathBuf::from("/h/.claude/.claude.json"),
            PathBuf::from("/h/.claude.json")
        ]
    );
    assert_eq!(
        global_config_candidates("/c".as_ref(), None),
        [PathBuf::from("/c/.claude.json")]
    );
}

#[test]
fn an_unreadable_candidate_falls_back_to_the_next_one() {
    let fs = MemFs::new()
        .unreadable("/h/.claude/.claude.json")
        .file(GLOBAL, r#"{"mcpServers": {"from-home": {}}}"#);
    let report = run(&fs, &quiet(), &request(false));
    assert_eq!(report.mcp_servers[0].name, "from-home");
    assert!(has_warning(&report, "denied"));
}

#[test]
fn malformed_json_is_a_warning_not_a_failure() {
    let fs = MemFs::new()
        .file(GLOBAL, "{ not json")
        .file("/h/proj/.mcp.json", "[1, 2")
        .file("/h/.claude/settings.json", "oops");
    let report = run(&fs, &quiet(), &request(true));
    assert!(report.mcp_servers.is_empty());
    assert_eq!(
        report
            .warnings
            .iter()
            .filter(|w| w.contains("malformed JSON"))
            .count(),
        3
    );
}

#[test]
fn disabled_project_servers_are_marked_from_any_settings_file() {
    let fs = MemFs::new()
        .file(
            "/h/proj/.mcp.json",
            r#"{"mcpServers": {"off1": {}, "off2": {}, "off3": {}, "on": {}}}"#,
        )
        .file(GLOBAL, r#"{"mcpServers": {"off1": {}}}"#)
        .file(
            "/h/.claude/settings.json",
            r#"{"disabledMcpjsonServers": ["off1"]}"#,
        )
        .file(
            "/h/proj/.claude/settings.json",
            r#"{"disabledMcpjsonServers": ["off2"]}"#,
        )
        .file(
            "/h/proj/.claude/settings.local.json",
            r#"{"disabledMcpjsonServers": ["off3"]}"#,
        );
    let report = run(&fs, &FakeUsage::ok(&[("on", 1, None)]), &request(true));
    let by_name = |name: &str| report.mcp_servers.iter().find(|s| s.name == name).unwrap();
    assert!(by_name("off2").disabled && by_name("off3").disabled);
    assert!(
        by_name("off2")
            .action
            .as_deref()
            .unwrap()
            .contains("remove it")
    );
    assert!(!by_name("on").disabled);
    // `off1` is defined at user scope too, but the project definition wins and is disabled.
    assert!(by_name("off1").disabled);
}

#[test]
fn unknown_usage_never_marks_a_server_unused() {
    let fs = MemFs::new().file(GLOBAL, r#"{"mcpServers": {"a": {}}}"#);
    let failing = run(&fs, &FakeUsage::failing(), &request(false));
    assert!(!failing.mcp_servers[0].unused);
    assert!(has_warning(&failing, "MCP usage is unknown"));
    let none = run(&fs, &FakeUsage::no_files(), &request(false));
    assert!(!none.mcp_servers[0].unused);
    assert_eq!(none.summary.mcp_servers_unused, 0);
    assert!(has_warning(&none, "cannot be judged"));
}

#[test]
fn the_window_start_follows_the_request() {
    let usage = quiet();
    let mut req = request(false);
    req.window_days = 7;
    run(&MemFs::new(), &usage, &req);
    assert_eq!(usage.requested.get(), Some(NOW - 7 * 86_400));
    assert_eq!(
        request(false).window_days,
        tokred_audit::DEFAULT_WINDOW_DAYS
    );
}
