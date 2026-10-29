**English** | [Português](routing.pt-BR.md)

# Model routing

With the [proxy](proxy.md) running, tokred can move work that does not need an expensive model to a cheaper one. One rule shapes everything below:

> **The main loop's model is never changed.** The model you chose for your conversation always answers it. tokred only routes requests you did not choose a model for, and only to a cheaper model.

## Who decides what

| Request | Can tokred change the model? |
|---|---|
| Main loop (your conversation) | **Never.** tokred only measures and [suggests](#suggestions-for-the-main-loop) |
| Subagent that inherited your model | Yes, once, on its first turn; the choice is kept for the rest of that subagent |
| Subagent whose model differs from the main loop's (for example one set in the agent's own definition, as the `tokred-*` agents do) | No: that model was chosen on purpose |
| Auxiliary request (for example the session title) | Yes, when it is cheaper. Kept as it is when it carries `cache_control` markers or tools, because it may share the main loop's prompt cache |
| Compaction | Never |

Requests are told apart by what they carry. Subagent requests have the `x-claude-code-agent-id` header. Compaction requests are recognised by the `x-claude-code-compaction` or `x-claude-code-request-class: compaction` header. Auxiliary requests have `x-claude-code-request-class: auxiliary`, or no agent id, no tools, no thinking and a single message, a shape the main loop never has. Anything else is the main loop.

## How a subagent is classified

The first rule that applies decides; when in doubt the model is kept.

1. A subagent seen for the first time after its first turn (for example after a proxy restart): kept.
2. A model that is not in tokred's model catalog: kept.
3. Explicit choice: a subagent whose model differs from the main loop's, or whose session has no main-loop request seen yet, is kept.
4. The `x-claude-code-agent-type` header, for `Explore` (a search task) and `Plan` (a planning task).
5. Tool set: only read-only tools (`Read`, `Grep`, `Glob`, `LS`, `WebFetch`, `WebSearch` and the like) mean a search-like task.
6. Keywords of the subagent's first message, in English and Portuguese.
7. Otherwise: kept.

Then the decision has to pass four checks, or the model is kept: the classification confidence reaches `min_confidence` (default 0.8); the tier of the task class (`[routing.tiers]`; by default search, summary and auxiliary requests are `cheap`, implementation is `mid`, the rest are kept) is not `keep`; a cheaper model of that tier supports the request; and the estimated saving is at least `min_saving_usd` (default 0.005).

The decision is made on the subagent's first turn and reused for all its turns: each subagent builds its own prompt cache, and switching in the middle would discard reasoning the new model cannot read.

## Safety

- **Only downgrades.** The target has to be in the model catalog, cheaper than the requested model (no price higher, the input price strictly lower), in `allowed_models` when that list is not empty, and able to handle everything the request uses (thinking mode, effort, context management, 1M context and other features). A request that uses something tokred does not recognise is kept.
- **Fallback.** If the API rejects a rewritten request (400, 404 or 422) before anything reached Claude Code, the original request is sent instead.
- **Rewriting stops for a subagent** when its requested model changes mid-run, when a later request needs something the cheaper model lacks, or when the API rejected a rewritten request. It is recorded as `fallback`, and the subagent is not switched again.
- **Circuit breaker.** When more than `[routing] max_rewrite_failure_rate` (default 5%) of the last 50 rewrites were rejected, rewriting stops for `[routing] cooldown` (default 30 minutes). It lives in the proxy's memory: a restart resets it.

## Modes and commands

| Command | Effect |
|---|---|
| `tokred routing status` | Mode and recent decisions with their estimated saving |
| `tokred routing explain <id>` | Why a decision was taken |
| `tokred routing on` | `enforce`: change the model of eligible requests |
| `tokred routing off` | `observe`: keep recording what routing would do |

The default is `observe`: decisions are recorded with their estimated saving and nothing changes, so you can check them before turning routing on. `[routing] mode = "off"` in `tokred.toml` records nothing. `tokred routing on` and `off` restart a running proxy to apply the mode; after editing `tokred.toml` yourself, run `tokred proxy restart`. Keys: `[routing]` and `[routing.tiers]` in [configuration](configuration.md).

## Suggestions for the main loop

tokred never changes your main model, but it tells you when a cheaper one would have done the job. A prompt is simple when it took at most 6 main-loop turns, 4,000 output tokens, 2,000 thinking tokens and one edit, and started no subagent. After 3 simple prompts in a row (`[suggest] min_simple_prompts`) tokred estimates what the next cheaper model would have cost, including rebuilding the prompt cache once, and suggests it when the saving is above $0.05 and the session had no suggestion in the last `[suggest] cooldown` (default 6 hours).

The suggestion appears in `tokred suggest` and `tokred stats` and, if you choose in `[suggest] channel`, in the status line or as a one-line message at a moment when switching is cheap: the main loop's prompt cache is already cold, right after `/compact`, or the proxy has seen no main-loop request in the session. `stats` never marks a suggestion as shown. Switching is always your call (`/model`).
