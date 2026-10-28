#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Snapshot of the `--json` shape of the report.

mod common;

use common::{FakeUsage, MemFs, NOW, request, run};
use serde_json::Value;

/// Paths are rendered with `/` so the snapshot is identical on every OS.
fn slashes(value: &mut Value) {
    match value {
        Value::String(text) => *text = text.replace('\\', "/"),
        Value::Array(items) => items.iter_mut().for_each(slashes),
        Value::Object(map) => map.values_mut().for_each(slashes),
        _ => {}
    }
}

#[test]
fn json_shape_is_stable() {
    let fs = MemFs::new()
        .file("/h/.claude/CLAUDE.md", "@rules/style.md\n<!-- note -->\nuser instructions\n")
        .file("/h/.claude/rules/style.md", "style rules")
        .file("/h/proj/CLAUDE.md", &"line\n".repeat(210))
        .file(
            "/h/.claude/skills/deploy/SKILL.md",
            "---\nname: deploy\ndescription: Deploys the app\n---\nSteps go here.\n",
        )
        .file(
            "/h/.claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews code\ntools: Read\n---\nReview carefully.\n",
        )
        .file(
            "/h/.claude.json",
            r#"{"mcpServers": {"github": {"type": "http", "url": "https://example.test"}, "old": {"command": "old-mcp"}},
                "projects": {"/h/proj": {"mcpServers": {"db": {"command": "db-mcp"}}}}}"#,
        )
        .file("/h/proj/.mcp.json", r#"{"mcpServers": {"old": {"command": "project-old"}}}"#)
        .file("/h/proj/.claude/settings.json", r#"{"disabledMcpjsonServers": ["old"]}"#);
    let usage = FakeUsage::ok(&[
        ("github", 12, Some(NOW - 3600)),
        ("plugin_x_y", 2, Some(NOW - 60)),
    ]);
    let report = run(&fs, &usage, &request(true));
    let mut json = serde_json::to_value(&report).unwrap();
    slashes(&mut json);
    insta::assert_json_snapshot!(json);
}
