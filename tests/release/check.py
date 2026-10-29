#!/usr/bin/env python3
"""Assertions on real state for the release smoke test (standard library only, Python 3.7+).

smoke.sh calls this file; every command prints `ok - <what was proven>` per assertion and, on the
first failure, `not ok - <what is wrong>` on stderr and exits 1. Run it as `python3 -I check.py`.
It reads files and databases and never touches the network; the one command that writes is `edit`.

  settings FILE [--hooks present|absent] [--bin PATH] [--user] [--url URL | --no-url | --foreign URL]
                [--equals FILE]
        --hooks    the six tokred hooks are in settings.json exactly once each (UserPromptSubmit,
                   PreToolUse Agent|Task, PreToolUse Read, PreToolUse Grep|Glob, Stop, SessionStart),
                   or none is. --bin also demands that they run that binary.
        --user     the starting document's own keys are still there: model=opus, env.FOO=bar and the
                   user's PreToolUse hook (matcher Bash, command `echo user-hook`), exactly once
        --url      env.ANTHROPIC_BASE_URL is that URL      --no-url   the key is absent
        --foreign  env.ANTHROPIC_BASE_URL is still that (someone else's) value
        --equals   the whole JSON document equals the one in FILE
  edit FILE (--drop-url | --set-url URL)   the test's own change to settings.json (the only writer)
  agents DIR (--count N | --none | --only NAME)   DIR/tokred-*.md files: exactly N (and the six known names), none,
        or only tokred-NAME.md (an agent the user edited, which unconfigure and uninstall keep)
  marker DB (--is VALUE | --empty)  meta.tokred_version of the SQLite database (empty: no row or '')
  toml FILE SECTION KEY VALUE       `KEY = VALUE` in [SECTION] of tokred.toml (VALUE as TOML text)
  sse FILE --text T --model M [--id-prefix msg_fake_]
        a complete SSE message: message_start .. message_stop, the text is T, stop_reason end_turn,
        and prints the message id on the last line (`id=<id>`)
  reply FILE --text T --model M     the non-streaming JSON message of the fake upstream
  sessions FILE --session ID --requests N --models A,B [--status active]
  turns FILE --session ID --content true|false --main-model M --main-sent T --main-text T
        --sub-model M --sub-sent T --sub-text T --agent-id ID
        the session detail of the dashboard API: two turns (main and subagent) with their models,
        origin, stop reason, usage, and `sent` / `response` (the new messages, the streamed text)
        or both null when content is not recorded
  sent FILE --session ID --expect ROLE=TEXT ... [--count N]   the newest turn carries only the new messages
  null-content FILE --session ID    every turn of the session has sent and response null
  stats FILE --requests N --p95-under MS            `tokred stats --json`: proxy.requests, overhead p95
  proxy-status FILE [--running true|false] [--port N]   `tokred proxy status --json`
"""

import argparse
import json
import os
import re
import subprocess
import sys

OK = 0


def ok(message):
    global OK
    OK += 1
    print("ok - %s" % message)


def fail(message):
    sys.stderr.write("not ok - %s\n" % message)
    sys.exit(1)


def expect(condition, good, bad):
    if condition:
        ok(good)
    else:
        fail(bad)


def load(path):
    try:
        with open(path, "r", encoding="utf-8-sig") as handle:  # tolerates the BOM Windows tools add
            return json.load(handle)
    except (OSError, ValueError) as error:
        fail("%s is not readable JSON: %s" % (path, error))


# ------------------------------------------------------------------ settings.json

SIX_HOOKS = [
    ("UserPromptSubmit", None, "route"),
    ("PreToolUse", "Agent|Task", "enforce"),
    ("PreToolUse", "Read", "guard-read"),
    ("PreToolUse", "Grep|Glob", "guard-search"),
    ("Stop", None, "session-check"),
    ("SessionStart", None, "session-start"),
]


def commands_of(entry):
    return [h.get("command", "") for h in entry.get("hooks", []) if isinstance(h, dict)]


def tokred_hook_entries(settings, event, name):
    """The entries of `event` whose command runs `tokred hook <name>`."""
    found = []
    for entry in (settings.get("hooks") or {}).get(event) or []:
        for command in commands_of(entry):
            if re.search(r"tokred(\.exe)?\"?\s+hook\s+%s(\s|$)" % re.escape(name), command):
                found.append((entry, command))
    return found


def cmd_settings(args):
    settings = load(args.file)
    if args.hooks == "present":
        for event, matcher, name in SIX_HOOKS:
            found = tokred_hook_entries(settings, event, name)
            expect(len(found) == 1, "settings.json has the tokred hook %s (%s) once" % (name, event),
                   "settings.json has %d tokred `hook %s` entries under %s, expected exactly 1" % (len(found), name, event))
            entry, command = found[0]
            if matcher is not None:
                expect(entry.get("matcher") == matcher, "the hook %s has the matcher %s" % (name, matcher),
                       "the hook %s has the matcher %r, expected %r" % (name, entry.get("matcher"), matcher))
            if args.bin:
                # Paths compared without regard to case or slash direction (Windows).
                norm = lambda text: text.replace("\\\\", "/").replace("\\", "/").lower()  # noqa: E731
                expect(norm(args.bin) in norm(command), "the hook %s runs %s" % (name, args.bin),
                       "the hook %s runs %r, expected it to contain %s" % (name, command, args.bin))
    elif args.hooks == "absent":
        every = [c for entries in (settings.get("hooks") or {}).values() for e in entries for c in commands_of(e)]
        left = [c for c in every if re.search(r"tokred(\.exe)?\"?\s+hook\s", c)]
        expect(not left, "settings.json has no tokred hook", "tokred hooks are still in settings.json: %r" % left)
    if args.user:
        expect(settings.get("model") == "opus", "settings.json keeps model=opus",
               "settings.json model is %r, expected 'opus'" % (settings.get("model"),))
        expect((settings.get("env") or {}).get("FOO") == "bar", "settings.json keeps env.FOO=bar",
               "settings.json env.FOO is %r, expected 'bar'" % ((settings.get("env") or {}).get("FOO"),))
        mine = [e for e in (settings.get("hooks") or {}).get("PreToolUse") or []
                if e.get("matcher") == "Bash" and "echo user-hook" in commands_of(e)]
        expect(len(mine) == 1, "settings.json keeps the user's own PreToolUse hook (Bash, echo user-hook) once",
               "the user's own PreToolUse hook is present %d times, expected 1" % len(mine))
    env = settings.get("env") or {}
    value = env.get("ANTHROPIC_BASE_URL")
    if args.url:
        expect(value == args.url, "env.ANTHROPIC_BASE_URL is %s" % args.url,
               "env.ANTHROPIC_BASE_URL is %r, expected %r" % (value, args.url))
    if args.no_url:
        expect(value is None, "env.ANTHROPIC_BASE_URL is absent", "env.ANTHROPIC_BASE_URL is still %r" % (value,))
    if args.foreign:
        expect(value == args.foreign, "the foreign ANTHROPIC_BASE_URL %s was left alone" % args.foreign,
               "the foreign ANTHROPIC_BASE_URL is now %r, expected it untouched (%r)" % (value, args.foreign))
    if args.equals:
        expected = load(args.equals)
        expect(settings == expected, "settings.json equals the starting document exactly",
               "settings.json differs from the starting document:\n  now:   %s\n  start: %s"
               % (json.dumps(settings, sort_keys=True), json.dumps(expected, sort_keys=True)))


def cmd_edit(args):
    """The test's own change to the user's settings.json (the only command here that writes)."""
    settings = load(args.file)
    env = settings.setdefault("env", {})
    if args.drop_url:
        env.pop("ANTHROPIC_BASE_URL", None)
    if args.set_url:
        env["ANTHROPIC_BASE_URL"] = args.set_url
    with open(args.file, "w", encoding="utf-8") as handle:
        json.dump(settings, handle, indent=2)
        handle.write("\n")
    ok("settings.json edited by the test (%s)" % ("url dropped" if args.drop_url else "url set to " + args.set_url))


# ------------------------------------------------------------------ agents, marker, toml

AGENTS = ["architect", "planner", "dev", "reviewer", "explorer", "summarizer"]


def cmd_agents(args):
    names = sorted(n for n in os.listdir(args.dir) if n.startswith("tokred-") and n.endswith(".md")) \
        if os.path.isdir(args.dir) else []
    if args.only:
        want = ["tokred-%s.md" % args.only]
        expect(names == want, "only the edited agent %s is left (kept on purpose, it was edited by the user)" % want[0],
               "the tokred agents left are %s, expected only %s" % (names, want))
        return
    if args.none:
        expect(not names, "no tokred agent file is left in %s" % args.dir, "tokred agents are left: %s" % names)
        return
    expect(len(names) == args.count, "%d tokred-*.md agent files are installed" % args.count,
           "%d tokred-*.md agent files in %s, expected %d: %s" % (len(names), args.dir, args.count, names))
    want = sorted("tokred-%s.md" % a for a in AGENTS)
    expect(names == want, "the agents are the six known ones", "the agents are %s, expected %s" % (names, want))


def marker_value(db):
    """meta.tokred_version, through the sqlite3 module or, where Python was built without it
    (some minimal images), through the sqlite3 command-line tool."""
    try:
        import sqlite3
    except ImportError:
        sqlite3 = None
    query = "select value from meta where key = 'tokred_version'"
    if sqlite3 is not None:
        try:
            connection = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
            try:
                row = connection.execute(query).fetchone()
            finally:
                connection.close()
            return row[0] if row else None
        except sqlite3.Error as error:
            fail("cannot read the meta table of %s: %s" % (db, error))
    try:
        done = subprocess.run(["sqlite3", "-readonly", db, query], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, universal_newlines=True)
    except OSError:
        fail("neither the Python sqlite3 module nor the sqlite3 tool is available")
    if done.returncode != 0:
        fail("cannot read the meta table of %s: %s" % (db, done.stderr.strip()))
    return done.stdout.rstrip("\n") if done.stdout.strip() != "" else None


def cmd_marker(args):
    value = marker_value(args.db)
    if args.empty:
        expect(value in (None, ""), "the database marker tokred_version is empty",
               "the database marker tokred_version is %r, expected it empty" % (value,))
    else:
        expect(value == args.is_, "the database marker tokred_version is %s" % args.is_,
               "the database marker tokred_version is %r, expected %r" % (value, args.is_))


def cmd_toml(args):
    section = None
    found = None
    with open(args.file, "r", encoding="utf-8") as handle:
        for line in handle:
            text = line.strip()
            match = re.match(r"^\[([^\]]+)\]$", text)
            if match:
                section = match.group(1)
                continue
            match = re.match(r"^([A-Za-z0-9_.-]+)\s*=\s*(.*?)\s*(#.*)?$", text)
            if match and section == args.section and match.group(1) == args.key:
                found = match.group(2)
    expect(found == args.value, "[%s] %s = %s" % (args.section, args.key, args.value),
           "[%s] %s is %r in %s, expected %r" % (args.section, args.key, found, args.file, args.value))


# ------------------------------------------------------------------ the fake upstream's replies

def read_text(path):
    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        return handle.read()


def sse_events(text):
    events = []
    for block in re.split(r"\r?\n\r?\n", text):
        name = None
        data = []
        for line in block.splitlines():
            if line.startswith("event:"):
                name = line[6:].strip()
            elif line.startswith("data:"):
                data.append(line[5:].strip())
        if name and data:
            try:
                events.append((name, json.loads("".join(data))))
            except ValueError:
                fail("an SSE event has data that is not JSON: %r" % "".join(data)[:80])
    return events


def cmd_sse(args):
    events = sse_events(read_text(args.file))
    names = [n for n, _ in events]
    expect(names and names[0] == "message_start" and names[-1] == "message_stop",
           "the reply is a whole SSE message (message_start .. message_stop)",
           "the reply is not a whole SSE message, events: %s" % names)
    start = events[0][1]["message"]
    expect(re.match(r"^%s\d+$" % re.escape(args.id_prefix), start["id"]), "the reply has a unique message id (%s)" % start["id"],
           "the message id %r does not look like %s<n>" % (start["id"], args.id_prefix))
    expect(start["model"] == args.model, "the reply names the model %s" % args.model,
           "the reply model is %r, expected %r" % (start["model"], args.model))
    deltas = [d["delta"]["text"] for n, d in events if n == "content_block_delta"]
    expect(len(deltas) >= 3, "the text arrives in %d streamed chunks" % len(deltas),
           "the text arrived in %d chunks, expected a stream of several" % len(deltas))
    expect("".join(deltas) == args.text, "the streamed text is the fake's: %r" % args.text,
           "the streamed text is %r, expected %r" % ("".join(deltas), args.text))
    stop = [d["delta"]["stop_reason"] for n, d in events if n == "message_delta"]
    expect(stop == ["end_turn"], "the stop reason is end_turn", "the stop reasons are %r" % stop)
    print("id=%s" % start["id"])


def cmd_reply(args):
    message = json.loads(read_text(args.file))
    expect(re.match(r"^msg_fake_\d+$", message.get("id", "")), "the JSON reply has a message id (%s)" % message.get("id"),
           "the reply id is %r" % message.get("id"))
    expect(message.get("model") == args.model, "the JSON reply names the model %s" % args.model,
           "the JSON reply model is %r, expected %r" % (message.get("model"), args.model))
    text = "".join(b.get("text", "") for b in message.get("content", []))
    expect(text == args.text, "the JSON reply text is the fake's: %r" % args.text,
           "the JSON reply text is %r, expected %r" % (text, args.text))
    print("id=%s" % message["id"])


# ------------------------------------------------------------------ dashboard API

def cmd_sessions(args):
    data = load(args.file)
    listed = [s for s in data.get("active", []) if s["id"] == args.session]
    status = args.status or "active"
    if status == "active":
        expect(len(listed) == 1, "the session %s is listed as active" % args.session,
               "the session %s is not in the active list: %s" % (args.session, json.dumps(data)[:300]))
    session = listed[0]
    expect(session.get("status") == "active", "its status is active", "its status is %r" % session.get("status"))
    expect(session["requests"] == args.requests, "it counts %d requests" % args.requests,
           "it counts %r requests, expected %d" % (session["requests"], args.requests))
    want = sorted(args.models.split(","))
    expect(sorted(session["models"]) == want, "it lists both models %s" % ", ".join(want),
           "its models are %r, expected %r" % (sorted(session["models"]), want))


def turn_of(turns, origin):
    mine = [t for t in turns if t["origin"] == origin]
    expect(len(mine) == 1, "the session has one %s turn" % origin,
           "the session has %d %s turns (origins: %s), expected 1" % (len(mine), origin, [t["origin"] for t in turns]))
    return mine[0]


def cmd_turns(args):
    detail = load(args.file)
    expect(detail["session"]["id"] == args.session, "the detail is of session %s" % args.session,
           "the detail is of session %r" % detail["session"]["id"])
    turns = detail["turns"]
    expect(len(turns) == 2, "the session detail has both turns", "the session detail has %d turns, expected 2" % len(turns))
    expect(turns[0]["ts"] <= turns[1]["ts"], "the turns are oldest first", "the turns are not oldest first")
    main = turn_of(turns, "main")
    sub = turn_of(turns, "subagent")
    expect(main["model_requested"] == args.main_model and main["model_effective"] == args.main_model,
           "the main turn ran on %s" % args.main_model, "the main turn models are %r/%r" % (main["model_requested"], main["model_effective"]))
    expect(sub["model_requested"] == args.sub_model and sub["model_effective"] == args.sub_model,
           "the subagent turn ran on %s" % args.sub_model, "the subagent turn models are %r/%r" % (sub["model_requested"], sub["model_effective"]))
    expect(sub["agent_id"] == args.agent_id, "the subagent turn carries the agent id %s" % args.agent_id,
           "the subagent turn agent_id is %r" % sub["agent_id"])
    expect(main["agent_id"] is None, "the main turn has no agent id", "the main turn agent_id is %r" % main["agent_id"])
    for label, turn in (("main", main), ("subagent", sub)):
        expect(turn["status"] == 200, "the %s turn answered 200" % label, "the %s turn status is %r" % (label, turn["status"]))
        expect(turn["stop_reason"] == "end_turn", "the %s turn stop reason is end_turn" % label,
               "the %s turn stop reason is %r" % (label, turn["stop_reason"]))
        expect(turn["usage"]["input"] == 120 and turn["usage"]["output"] == 15,
               "the %s turn used 120 input and 15 output tokens" % label, "the %s turn usage is %r" % (label, turn["usage"]))
        expect(turn["in_flight"] is False, "the %s turn is finished" % label, "the %s turn is still in flight" % label)
    if args.content == "true":
        sent = [(m["role"], m["text"]) for m in main["sent"] or []]
        expect(sent == [("system", args.system), ("user", args.main_sent)],
               "the main turn sent the system prompt and the user message",
               "the main turn sent %r" % (sent,))
        sent = [(m["role"], m["text"]) for m in sub["sent"] or []]
        expect(sent == [("user", args.sub_sent)], "the subagent turn sent only its own message", "the subagent turn sent %r" % (sent,))
        expect((main["response"] or {}).get("text") == args.main_text, "the main response text is the streamed one: %r" % args.main_text,
               "the main response is %r" % (main["response"],))
        expect((sub["response"] or {}).get("text") == args.sub_text, "the subagent response text is the fake's: %r" % args.sub_text,
               "the subagent response is %r" % (sub["response"],))
    else:
        for label, turn in (("main", main), ("subagent", sub)):
            expect(turn["sent"] is None and turn["response"] is None, "the %s turn has sent and response null" % label,
                   "the %s turn recorded content although recording is off: %r" % (label, (turn["sent"], turn["response"])))


def cmd_sent(args):
    detail = load(args.file)
    turns = detail["turns"]
    expect(len(turns) == args.count, "the session has %d turns" % args.count, "the session has %d turns, expected %d" % (len(turns), args.count))
    newest = turns[-1]
    sent = ["%s=%s" % (m["role"], m["text"]) for m in newest["sent"] or []]
    expect(sent == args.expect, "the newest turn sent only the messages that are new: %s" % ", ".join(args.expect),
           "the newest turn sent %r, expected only the new messages %r (the history was already seen)" % (sent, args.expect))


def cmd_null_content(args):
    detail = load(args.file)
    turns = detail["turns"]
    expect(turns, "the session has turns", "the session has no turns")
    for turn in turns:
        expect(turn["sent"] is None and turn["response"] is None, "turn %s has sent and response null" % turn["id"],
               "turn %s kept content with recording off: %r" % (turn["id"], (turn["sent"], turn["response"])))
        expect(turn["usage"]["input"] == 120, "turn %s kept its metadata (tokens)" % turn["id"], "turn %s lost its usage" % turn["id"])


# ------------------------------------------------------------------ stats and proxy status

def cmd_stats(args):
    proxy = load(args.file).get("proxy")
    expect(proxy is not None, "tokred stats --json has a proxy section", "tokred stats --json has no proxy section")
    expect(proxy["requests"] == args.requests, "the Proxy section counts %d requests" % args.requests,
           "the Proxy section counts %r requests, expected %d" % (proxy["requests"], args.requests))
    p95 = proxy["overhead"]["p95_ms"]
    expect(p95 < args.p95_under, "the proxy overhead p95 is %.3f ms (under %g ms)" % (p95, args.p95_under),
           "the proxy overhead p95 is %.3f ms, over the %g ms budget" % (p95, args.p95_under))


def cmd_proxy_status(args):
    status = load(args.file)
    want = args.running == "true"
    expect(status.get("running") is want, "tokred proxy status --json says running: %s" % args.running,
           "tokred proxy status --json says running: %r" % status.get("running"))
    if args.port:
        expect(str(status.get("url", "")).endswith(":%d" % args.port), "the proxy url is on port %d" % args.port,
               "the proxy url is %r, expected port %d" % (status.get("url"), args.port))


# ------------------------------------------------------------------ wiring

def build():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command")
    sub.required = True

    p = sub.add_parser("settings")
    p.add_argument("file")
    p.add_argument("--hooks", choices=["present", "absent"])
    p.add_argument("--bin")
    p.add_argument("--user", action="store_true")
    p.add_argument("--url")
    p.add_argument("--no-url", action="store_true")
    p.add_argument("--foreign")
    p.add_argument("--equals")
    p.set_defaults(run=cmd_settings)

    p = sub.add_parser("edit")
    p.add_argument("file")
    p.add_argument("--drop-url", action="store_true")
    p.add_argument("--set-url")
    p.set_defaults(run=cmd_edit)

    p = sub.add_parser("agents")
    p.add_argument("dir")
    p.add_argument("--count", type=int, default=6)
    p.add_argument("--none", action="store_true")
    p.add_argument("--only", help="exactly tokred-NAME.md is left (an agent the user edited)")
    p.set_defaults(run=cmd_agents)

    p = sub.add_parser("marker")
    p.add_argument("db")
    p.add_argument("--is", dest="is_")
    p.add_argument("--empty", action="store_true")
    p.set_defaults(run=cmd_marker)

    p = sub.add_parser("toml")
    for name in ("file", "section", "key", "value"):
        p.add_argument(name)
    p.set_defaults(run=cmd_toml)

    p = sub.add_parser("sse")
    p.add_argument("file")
    p.add_argument("--text", required=True)
    p.add_argument("--model", required=True)
    p.add_argument("--id-prefix", default="msg_fake_")
    p.set_defaults(run=cmd_sse)

    p = sub.add_parser("reply")
    p.add_argument("file")
    p.add_argument("--text", required=True)
    p.add_argument("--model", required=True)
    p.set_defaults(run=cmd_reply)

    p = sub.add_parser("sessions")
    p.add_argument("file")
    p.add_argument("--session", required=True)
    p.add_argument("--requests", type=int, required=True)
    p.add_argument("--models", required=True)
    p.add_argument("--status")
    p.set_defaults(run=cmd_sessions)

    p = sub.add_parser("turns")
    p.add_argument("file")
    p.add_argument("--session", required=True)
    p.add_argument("--content", choices=["true", "false"], required=True)
    p.add_argument("--agent-id", required=True)
    for name in ("main-model", "sub-model"):
        p.add_argument("--" + name, required=True)
    for name in ("system", "main-sent", "main-text", "sub-sent", "sub-text"):
        p.add_argument("--" + name, default="")
    p.set_defaults(run=cmd_turns)

    p = sub.add_parser("sent")
    p.add_argument("file")
    p.add_argument("--session", required=True)
    p.add_argument("--expect", action="append", required=True, metavar="ROLE=TEXT")
    p.add_argument("--count", type=int, default=2)
    p.set_defaults(run=cmd_sent)

    p = sub.add_parser("null-content")
    p.add_argument("file")
    p.add_argument("--session", required=True)
    p.set_defaults(run=cmd_null_content)

    p = sub.add_parser("stats")
    p.add_argument("file")
    p.add_argument("--requests", type=int, required=True)
    p.add_argument("--p95-under", type=float, required=True)
    p.set_defaults(run=cmd_stats)

    p = sub.add_parser("proxy-status")
    p.add_argument("file")
    p.add_argument("--running", choices=["true", "false"], default="true")
    p.add_argument("--port", type=int)
    p.set_defaults(run=cmd_proxy_status)
    return parser


def main():
    args = build().parse_args()
    try:
        args.run(args)
    except SystemExit:
        raise
    except Exception as error:  # a malformed document is a failed assertion, not a traceback
        fail("%s: %s: %s" % (args.command, type(error).__name__, error))


if __name__ == "__main__":
    main()
