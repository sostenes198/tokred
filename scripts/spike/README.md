**English** | [Português](README.pt-BR.md)

# Proxy spike (phase F7)

`proxy_probe.py` is a **throwaway** pass-through proxy used to answer the open questions of the spec before the real `tokred-proxy` is built. It is not part of tokred and is not shipped. It never logs credential values or message contents: only header names, `x-claude-code-*` values, `anthropic-beta`/`anthropic-version`, structural facts about the body (model, stream, counts) and the usage of each response, to `/tmp/tokred-spike/log.jsonl`.

```sh
mkdir -p /tmp/tokred-spike
python3 -I scripts/spike/proxy_probe.py &      # prints the port
ANTHROPIC_BASE_URL=http://127.0.0.1:<port> claude -p "Reply with exactly one word: ok" --model haiku < /dev/null
```

Probe: rewrite the model of **subagent** requests only (the main loop is never touched):

```sh
SPIKE_REWRITE="sonnet:claude-haiku-5-5" SPIKE_AGENT_ONLY=1 python3 -I scripts/spike/proxy_probe.py &
```

## Testing a claude.ai subscription without touching your login

Use a throwaway Claude Code config folder so your real login is not changed:

```sh
export CLAUDE_CONFIG_DIR=$(mktemp -d)
env -u ANTHROPIC_API_KEY claude auth login --claudeai      # opens the browser
env -u ANTHROPIC_API_KEY claude auth status                # authMethod must NOT be "api_key"
python3 -I scripts/spike/proxy_probe.py &                  # note the port
env -u ANTHROPIC_API_KEY ANTHROPIC_BASE_URL=http://127.0.0.1:<port> \
  claude -p "Reply with exactly one word: ok" --model haiku < /dev/null
cat /tmp/tokred-spike/log.jsonl                            # status 200? which headers? which betas?
```

Then repeat with `SPIKE_REWRITE`/`SPIKE_AGENT_ONLY` and the Explore-subagent prompt to see whether subscription traffic tolerates a rewritten model. Record the result in `docs/decisions.md`. Remove the throwaway folder afterwards.
