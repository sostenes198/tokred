//! Realistic stdin payloads for `tokred doctor --bench`, which times real process spawns.
//!
//! The paths do not exist on purpose: the payloads exercise the full hook path (parse, store
//! open, decision, record) without depending on the machine they run on.

use serde_json::json;
use tokred_core::HookEvent;

const SESSION: &str = "tokred-bench-session";
const CWD: &str = "/tokred-bench/project";
const TRANSCRIPT: &str = "/tokred-bench/transcript.jsonl";

fn base(event_name: &str) -> serde_json::Value {
    json!({
        "session_id": SESSION,
        "transcript_path": TRANSCRIPT,
        "cwd": CWD,
        "permission_mode": "default",
        "hook_event_name": event_name,
    })
}

fn pre_tool_use(tool: &str, input: serde_json::Value) -> String {
    let mut value = base("PreToolUse");
    value["tool_name"] = json!(tool);
    value["tool_input"] = input;
    value["tool_use_id"] = json!("toolu_bench");
    value.to_string()
}

/// One or more stdin payloads per hook, in the shape Claude Code sends them.
#[must_use]
pub fn bench_fixtures() -> Vec<(HookEvent, String)> {
    let mut start = base("SessionStart");
    start["source"] = json!("startup");
    let mut prompt = base("UserPromptSubmit");
    prompt["prompt"] =
        json!("Implement pagination on the users list endpoint in src/users/list.ts and add tests");
    let mut stop = base("Stop");
    stop["stop_hook_active"] = json!(false);
    let mut switch = base("PreModelSwitch");
    switch["from_model"] = json!("claude-sonnet-5");
    switch["to_model"] = json!("claude-opus-5");
    switch["requested_model"] = json!("opus");
    switch["source"] = json!("command");
    switch["context_tokens"] = json!(182_340);
    switch["prompt_cache_warm"] = json!(true);
    switch["cache_ttl"] = json!("5m");
    switch["estimated_cache_write_usd"] = json!(1.1396);

    vec![
        (HookEvent::SessionStart, start.to_string()),
        (HookEvent::Route, prompt.to_string()),
        (
            HookEvent::Enforce,
            pre_tool_use(
                "Agent",
                json!({"subagent_type": "tokred-dev", "description": "Add pagination", "prompt": "Add pagination to src/users/list.ts"}),
            ),
        ),
        (
            HookEvent::GuardRead,
            pre_tool_use(
                "Read",
                json!({"file_path": "/tokred-bench/project/Cargo.lock"}),
            ),
        ),
        (
            HookEvent::GuardRead,
            pre_tool_use(
                "Read",
                json!({"file_path": "/tokred-bench/project/src/main.rs", "offset": 1, "limit": 200}),
            ),
        ),
        (
            HookEvent::GuardSearch,
            pre_tool_use(
                "Grep",
                json!({"pattern": "fn main", "path": "/tokred-bench/project/src", "output_mode": "content"}),
            ),
        ),
        (
            HookEvent::GuardSearch,
            pre_tool_use("Glob", json!({"pattern": "**/*"})),
        ),
        (HookEvent::SessionCheck, stop.to_string()),
        (HookEvent::ModelGuard, switch.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hook_has_a_valid_json_fixture() {
        let fixtures = bench_fixtures();
        for event in HookEvent::ALL {
            assert!(fixtures.iter().any(|(e, _)| *e == event), "{event:?}");
        }
        for (event, stdin) in &fixtures {
            let value: serde_json::Value = serde_json::from_str(stdin).unwrap();
            assert_eq!(value["session_id"], SESSION);
            assert_eq!(value["cwd"], CWD, "{event:?}");
        }
    }

    #[test]
    fn pre_tool_use_fixtures_name_their_tool() {
        for (event, stdin) in bench_fixtures() {
            if event.claude_event() == "PreToolUse" {
                let value: serde_json::Value = serde_json::from_str(&stdin).unwrap();
                assert!(value["tool_name"].is_string());
                assert!(value["tool_input"].is_object());
            }
        }
    }
}
