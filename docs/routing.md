**English** | [Português](routing.pt-BR.md)

# Model routing

With the [proxy](proxy.md) running, tokred can move work that does not need an expensive model to a cheaper one. The rule that shapes everything below:

> **The main loop's model is never changed.** The model you chose for your conversation is always the one that answers it. tokred only routes requests you did not choose a model for, and only to a cheaper model.

## Who decides what

| Request | Can tokred change the model? |
|---|---|
| Main loop (your conversation) | **Never.** tokred only measures and [suggests](#suggestions-for-the-main-loop) |
| Subagent that inherited your model | Yes, once, on its first turn, and the choice is kept for the rest of that subagent |
| Subagent with its own model (frontmatter, `CLAUDE_CODE_SUBAGENT_MODEL`, a `tokred-*` agent) | No: that model was chosen on purpose |
| Auxiliary request (e.g. the session title) | Yes, when it is cheaper, including the cost of the prompt cache |
| Compaction | No (summary quality affects the rest of the session) |

How tokred tells them apart: subagent requests carry the `x-claude-code-agent-id` header. Auxiliary requests have no tools, no thinking and a single message, a shape the main loop never has. When a future Claude Code sends the documented `x-claude-code-request-class` header, it is used too.

## How a subagent is classified

The first layer that is confident enough decides; when in doubt the model is kept.

1. Headers, when present (agent type).
2. Explicit choice: a subagent whose model differs from the main loop's was pinned on purpose and is kept.
3. Tool set: only read and search tools means a search-like task.
4. Keywords of the subagent's task, in English and Portuguese.
5. Otherwise: keep.

The decision is made on the subagent's first turn and reused for all its turns: each subagent builds its own prompt cache, and switching in the middle would discard reasoning the new model cannot read.

## Safety

- **Only downgrades**, only to a model listed in the catalog with every capability the request uses (thinking mode, effort, context management, 1M context). Otherwise the model is kept.
- **Fallback:** if the API rejects a rewritten request before anything reached Claude Code, the original is sent instead.
- **Circuit breaker:** too many rejected rewrites turn routing off for a while; `tokred routing status` shows it.

## Modes

```sh
tokred routing status            # mode, circuit breaker, recent decisions
tokred routing explain <id>      # why a decision was taken
tokred routing on                # enforce: change the model of eligible requests
tokred routing off               # observe: keep recording what routing would do
```

The default is `observe`: decisions are recorded with their estimated saving and nothing changes, so you can check them before turning routing on.

## Suggestions for the main loop

tokred never changes your main model, but it tells you when a cheaper one would have done the job: after several simple prompts in a row (few turns, small output, little or no editing), it estimates what the cheaper model would have cost, including rebuilding the prompt cache, and shows the suggestion in `tokred suggest`, `tokred stats` and, if you choose, in the status line or as a one-line message at a moment when the cache is already cold. Switching is always your call (`/model`).

## Configuration

`[routing]`, `[routing.tiers]` and `[suggest]` in `tokred.toml`; see [configuration](configuration.md).
